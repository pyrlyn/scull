/*
 * Copyright (c) 2026 Ivan Tugay
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The C ABI as a host uses it, from two threads: one feeds output and
 * resizes, the other updates a frame and reads every byte the view points
 * to. Built and run under the sanitizers by `just c-abi-test`; a sanitizer
 * report or a wrong status fails it.
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
};

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

static tt_status feed_all(tt_term *term) {
    char line[LINE_BYTES];
    for (int i = 0; i < FEEDS; i++) {
        /* SGR colour, a wide CJK character and a bell, as a shell would. */
        int len = snprintf(line, sizeof line,
                           "\x1b[3%dmline %d \xe4\xb8\xad\x1b[0m\a\r\n",
                           i % 8, i);
        CHECK(tt_term_feed(term, (const uint8_t *)line, (size_t)len));
        if (i % RESIZE_EVERY == 0) {
            uint16_t cols = (uint16_t)(COLS - (i / RESIZE_EVERY) % WIDTH_STEPS);
            CHECK(tt_term_resize_begin(term));
            CHECK(tt_term_resize(term, cols, ROWS, 0, 0));
        }
    }
    CHECK(tt_term_resize(term, COLS, ROWS, 0, 0));
    return TT_OK;
}

static void *feeder(void *arg) {
    struct shared *shared = arg;
    shared->feeder_status = feed_all(shared->term);
    atomic_store(&shared->feeding_done, 1);
    return NULL;
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

static tt_status draw_until_done(struct shared *shared) {
    tt_frame *frame = tt_frame_new();
    if (frame == NULL)
        return TT_INVALID;
    tt_frame_view view;
    unsigned long sum = 0;
    unsigned long frames = 0;
    tt_status status = TT_OK;
    while (status == TT_OK && !atomic_load(&shared->feeding_done)) {
        status = update(frame, shared->term, &view, &sum);
        frames++;
    }
    /* The last line fed is on the screen once the feeder is done. */
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
    tt_status drawn = draw_until_done(&shared);
    pthread_join(thread, NULL);
    tt_status bells = bells_coalesce(shared.term);
    tt_term_free(shared.term);

    if (shared.feeder_status != TT_OK || drawn != TT_OK || bells != TT_OK) {
        fprintf(stderr, "feeder %d, drawer %d, events %d\n",
                (int)shared.feeder_status, (int)drawn, (int)bells);
        return 1;
    }
    puts("ok");
    return 0;
}
