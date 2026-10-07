/*
 * Copyright (c) 2026 Ivan Tugay
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The C ABI as a host uses it, from two threads: one feeds output, images,
 * resizes and input, the other updates a frame and reads every byte the view
 * points to, image pixels included. One image is kept past the frame and
 * the terminal, as a texture cache would. Built and run under the
 * sanitizers by `just c-abi-test`; a sanitizer report or a wrong status
 * fails it.
 */

#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>

#include "scull.h"

enum {
    COLS = 80,
    ROWS = 24,
    FEEDS = 3000,
    /* Resize every so many feeds, so frames see new sizes mid-stream. */
    RESIZE_EVERY = 400,
    /* Narrowest width the feeder resizes to: COLS - WIDTH_STEPS + 1. */
    WIDTH_STEPS = 7,
    LINE_BYTES = 96,
    /* Feed an image every so many lines, away from the end so the last
     * line still sits where the check expects it. */
    IMAGE_EVERY = 100,
    IMAGE_AT = 50,
    /* Cell size in pixels the feeder reports with each resize. */
    CELL_W = 8,
    CELL_H = 16,
    /* The kitty image is 2 x 2 black pixels. */
    KITTY_SIDE = 2,
};

/* A sixel image one cell tall: the cursor moves under it. */
static const char SIXEL[] = "\x1bPq#0;2;100;0;0#0~~-~~\x1b\\";
/* A kitty image shown without moving the cursor or replying. */
static const char KITTY[] =
    "\x1b_Gi=1,f=24,s=2,v=2,a=T,C=1,q=2;AAAAAAAAAAAAAAAA\x1b\\";

#define CHECK(call)                                                    \
    do {                                                               \
        tt_status status_ = (call);                                    \
        if (status_ != TT_OK) {                                        \
            fprintf(stderr, "%s:%d: %s answered %d\n", __FILE__,       \
                    __LINE__, #call, (int)status_);                    \
            return status_;                                            \
        }                                                              \
    } while (0)

struct shared {
    tt_term *term;
    atomic_int feeding_done;
    tt_status feeder_status;
};

/* Input on a terminal with no child: encoded under the lock the drawer
 * takes too, then refused for want of a child. */
static tt_status send_input(tt_term *term) {
    tt_key_event key;
    memset(&key, 0, sizeof key);
    key.struct_size = sizeof key;
    key.key = 'a';
    key.action = TT_KEY_PRESS;
    key.text.ptr = (const uint8_t *)"a";
    key.text.len = 1;
    if (tt_term_key(term, &key) != TT_CLOSED)
        return TT_INVALID;
    tt_mouse_event wheel;
    memset(&wheel, 0, sizeof wheel);
    wheel.struct_size = sizeof wheel;
    wheel.action = TT_MOUSE_PRESS;
    wheel.button = TT_MOUSE_WHEEL_UP;
    uint8_t taken = 1;
    /* Untracked on the main screen: the host's to scroll with. */
    CHECK(tt_term_mouse(term, &wheel, &taken));
    if (taken != 0)
        return TT_INVALID;
    CHECK(tt_term_scroll_display(term, ROWS));
    CHECK(tt_term_scroll_display(term, -ROWS));
    return TT_OK;
}

static tt_status feed_all(tt_term *term) {
    char line[LINE_BYTES];
    for (int i = 0; i < FEEDS; i++) {
        /* SGR colour, a wide CJK character and a bell, as a shell would. */
        int len = snprintf(line, sizeof line,
                           "\x1b[3%dmline %d \xe4\xb8\xad\x1b[0m\a\r\n",
                           i % 8, i);
        CHECK(tt_term_feed(term, (const uint8_t *)line, (size_t)len));
        if (i % IMAGE_EVERY == IMAGE_AT) {
            const char *image = i % (2 * IMAGE_EVERY) == IMAGE_AT ? SIXEL : KITTY;
            CHECK(tt_term_feed(term, (const uint8_t *)image, strlen(image)));
        }
        if (i % RESIZE_EVERY == 0) {
            CHECK(send_input(term));
            uint16_t cols = (uint16_t)(COLS - (i / RESIZE_EVERY) % WIDTH_STEPS);
            CHECK(tt_term_resize_begin(term));
            CHECK(tt_term_resize(term, cols, ROWS, (uint16_t)(cols * CELL_W),
                                 ROWS * CELL_H));
        }
    }
    CHECK(tt_term_resize(term, COLS, ROWS, 0, 0));
    /* On the last row, so the final frame has an image to keep. */
    CHECK(tt_term_feed(term, (const uint8_t *)KITTY, strlen(KITTY)));
    return TT_OK;
}

static void *feeder(void *arg) {
    struct shared *shared = arg;
    shared->feeder_status = feed_all(shared->term);
    atomic_store(&shared->feeding_done, 1);
    return NULL;
}

/* Reads every pixel of an image. */
static unsigned long touch_image(const tt_image *image, uint32_t *width,
                                 uint32_t *height) {
    size_t stride = 0;
    const uint8_t *pixels = tt_image_pixels(image, width, height, &stride);
    unsigned long sum = 0;
    for (size_t i = 0; pixels != NULL && i < stride * *height; i++)
        sum += pixels[i];
    return sum;
}

/* Reads every byte a view points to, so a dangling pointer is caught. */
static unsigned long touch(const tt_frame_view *view) {
    unsigned long sum = 0;
    for (size_t i = 0; i < view->cells_len; i++)
        sum += view->cells[i].codepoint + view->cells[i].style;
    for (size_t r = 0; r < view->lines_len; r++) {
        const tt_row *row = &view->lines[r];
        for (size_t i = 0; i < row->runs_len; i++)
            sum += row->runs[i].text_len;
        for (size_t i = 0; i < row->text_len; i++)
            sum += row->text[i];
    }
    for (size_t i = 0; i < view->styles_len; i++)
        sum += view->styles[i].fg;
    for (size_t i = 0; i < view->scrolls_len; i++)
        sum += view->scrolls[i].from;
    for (size_t i = 0; i < view->dirty_len; i++)
        sum += view->dirty[i];
    for (size_t i = 0; i < view->placements_len; i++) {
        const tt_placement *p = &view->placements[i];
        uint32_t width = 0, height = 0;
        /* A host's texture upload: its own reference while it reads. */
        tt_image_retain(p->image);
        sum += touch_image(p->image, &width, &height) + p->generation;
        tt_image_release(p->image);
    }
    return sum;
}

static tt_status update(tt_frame *frame, tt_term *term, tt_frame_view *view,
                        unsigned long *sum) {
    memset(view, 0, sizeof *view);
    view->struct_size = sizeof *view;
    CHECK(tt_frame_update(frame, term, view));
    if ((size_t)view->cols * view->rows != view->cells_len ||
        view->lines_len != view->rows) {
        fprintf(stderr, "view of %ux%u has %zu cells, %zu rows\n",
                view->cols, view->rows, view->cells_len, view->lines_len);
        return TT_INVALID;
    }
    *sum += touch(view);
    return TT_OK;
}

/* Draws until the feeder is done, then retains the image on the last row
 * into `*kept`. */
static tt_status draw_until_done(struct shared *shared, const tt_image **kept) {
    tt_frame *frame = tt_frame_new();
    if (frame == NULL)
        return TT_INVALID;
    tt_frame_view view;
    unsigned long sum = 0;
    unsigned long frames = 0;
    tt_status status = TT_OK;
    /* Composing while output streams in: the overlay follows the cursor. */
    static const char COMPOSING[] = "\xe6\x97\xa5\xe6\x9c\xac";
    while (status == TT_OK && !atomic_load(&shared->feeding_done)) {
        size_t len = (frames & 1) ? sizeof COMPOSING - 1 : 0;
        status = tt_frame_preedit(frame, (const uint8_t *)COMPOSING, len, 3);
        if (status == TT_OK)
            status = update(frame, shared->term, &view, &sum);
        frames++;
    }
    /* The last line fed is on the screen once the feeder is done. */
    if (status == TT_OK)
        status = tt_frame_preedit(frame, NULL, 0, 0);
    if (status == TT_OK)
        status = update(frame, shared->term, &view, &sum);
    if (status == TT_OK) {
        const tt_row *last = &view.lines[ROWS - 2];
        char expected[LINE_BYTES];
        int len = snprintf(expected, sizeof expected, "line %d", FEEDS - 1);
        if (last->text_len < (size_t)len ||
            memcmp(last->text, expected, (size_t)len) != 0) {
            fprintf(stderr, "last line is %.*s\n", (int)last->text_len,
                    (const char *)last->text);
            status = TT_INVALID;
        }
    }
    for (size_t i = 0; status == TT_OK && i < view.placements_len; i++) {
        if (view.placements[i].row == ROWS - 1) {
            *kept = view.placements[i].image;
            tt_image_retain(*kept);
        }
    }
    tt_frame_free(frame);
    printf("%lu frames drawn, checksum %lu\n", frames, sum);
    return status;
}

static tt_status bells_coalesce(tt_term *term) {
    tt_event event;
    memset(&event, 0, sizeof event);
    event.struct_size = sizeof event;
    CHECK(tt_term_poll_event(term, &event));
    if (event.kind != TT_EVENT_BELL)
        return TT_INVALID;
    return tt_term_poll_event(term, &event) == TT_EMPTY ? TT_OK : TT_INVALID;
}

int main(void) {
    if (tt_abi_version() != TT_ABI_VERSION) {
        fprintf(stderr, "library ABI %u, header %u\n", tt_abi_version(),
                TT_ABI_VERSION);
        return 1;
    }
    tt_term_options options;
    memset(&options, 0, sizeof options);
    options.struct_size = sizeof options;
    options.abi_version = TT_ABI_VERSION;
    options.cols = COLS;
    options.rows = ROWS;
    options.scrollback = 1000;
    struct shared shared = {.term = NULL, .feeder_status = TT_OK};
    atomic_init(&shared.feeding_done, 0);
    if (tt_term_new(&options, &shared.term) != TT_OK)
        return 1;

    pthread_t thread;
    if (pthread_create(&thread, NULL, feeder, &shared) != 0)
        return 1;
    const tt_image *kept = NULL;
    tt_status drawn = draw_until_done(&shared, &kept);
    pthread_join(thread, NULL);
    tt_status bells = bells_coalesce(shared.term);
    tt_term_free(shared.term);

    /* The retained image outlives the frame and the terminal: black and
     * opaque, so its bytes sum to the alpha channel alone. */
    uint32_t width = 0, height = 0;
    unsigned long kept_sum = touch_image(kept, &width, &height);
    tt_image_release(kept);
    unsigned long opaque = KITTY_SIDE * KITTY_SIDE * 255UL;
    tt_status image = kept != NULL && width == KITTY_SIDE &&
                              height == KITTY_SIDE && kept_sum == opaque
                          ? TT_OK
                          : TT_INVALID;

    if (shared.feeder_status != TT_OK || drawn != TT_OK || bells != TT_OK ||
        image != TT_OK) {
        fprintf(stderr, "feeder %d, drawer %d, events %d, image %d\n",
                (int)shared.feeder_status, (int)drawn, (int)bells, (int)image);
        return 1;
    }
    puts("ok");
    return 0;
}
