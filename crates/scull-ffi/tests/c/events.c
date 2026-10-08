/*
 * Copyright (c) 2026 Ivan Tugay
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The event calls as a host uses them, from two threads: one feeds titles,
 * directories, shell marks, clipboard writes and reads, hyperlinks and
 * notifications, the other polls them, copies out their text, answers or
 * denies every clipboard read and resolves link ids. Then, on one thread,
 * each kind is checked for its exact text. Built and run under the
 * sanitizers by `just c-abi-test`; a sanitizer report or a wrong status
 * fails it.
 */

#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "scull.h"

enum {
    COLS = 40,
    ROWS = 10,
    /* Past the core's 1024 link ids, so the sweep reclaims them meanwhile. */
    FEEDS = 3000,
    LINE_BYTES = 256,
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

static tt_status feed(tt_term *term, const char *bytes) {
    return tt_term_feed(term, (const uint8_t *)bytes, strlen(bytes));
}

static tt_status feed_all(tt_term *term) {
    char line[LINE_BYTES];
    for (int i = 0; i < FEEDS; i++) {
        int len = snprintf(line, sizeof line,
                           "\x1b]2;title %d\a\x1b]7;file://h/%d\a"
                           "\x1b]133;D;%d\a\x1b]52;c;aGk=\a\x1b]52;c;?\a"
                           "\x1b]8;;http://x/%d\x1b\\%d\x1b]8;;\x1b\\"
                           "\x1b]777;notify;n%d;body\a\r\n",
                           i, i, i % 256, i, i, i);
        CHECK(tt_term_feed(term, (const uint8_t *)line, (size_t)len));
    }
    return TT_OK;
}

static void *feeder(void *arg) {
    struct shared *shared = arg;
    shared->feeder_status = feed_all(shared->term);
    atomic_store(&shared->feeding_done, 1);
    return NULL;
}

/* Copies one part of an event's text: asks for the length, then reads it
 * into a buffer of exactly that size. */
static tt_status event_text(tt_term *term, const tt_event *event, uint32_t part,
                            uint8_t **out, size_t *out_len) {
    size_t len = 0;
    tt_status status = tt_term_event_text(term, event->serial, part, NULL, 0, &len);
    if (status == TT_OK && len == 0) {
        *out = NULL;
        *out_len = 0;
        return TT_OK;
    }
    if (status != TT_FULL)
        return status;
    uint32_t want = part == TT_EVENT_TEXT_TITLE ? event->title_len : event->text_len;
    if (len != want)
        return TT_INVALID;
    uint8_t *buf = malloc(len);
    if (buf == NULL)
        return TT_INVALID;
    status = tt_term_event_text(term, event->serial, part, buf, len, out_len);
    if (status != TT_OK) {
        free(buf);
        return status;
    }
    *out = buf;
    return TT_OK;
}

/* Handles one polled event as a host would. */
static tt_status handle(tt_term *term, const tt_event *event, unsigned long *reads) {
    uint8_t *text = NULL;
    size_t len = 0;
    CHECK(event_text(term, event, TT_EVENT_TEXT_BODY, &text, &len));
    uint8_t *title = NULL;
    size_t title_len = 0;
    tt_status status = event_text(term, event, TT_EVENT_TEXT_TITLE, &title, &title_len);
    if (status == TT_OK && event->kind == TT_EVENT_CLIPBOARD_READ) {
        /* Alternate answers and refusals; a full reply queue is fine. */
        if ((*reads)++ % 2 == 0) {
            status = tt_term_clipboard_reply(term, event->id, (const uint8_t *)"ok", 2);
            if (status == TT_FULL)
                status = TT_OK;
        } else {
            status = tt_term_clipboard_deny(term, event->id);
        }
    }
    if (status == TT_OK && event->kind == TT_EVENT_LINK) {
        /* The row may have scrolled away and the id been swept since. */
        uint8_t uri[64];
        size_t uri_len = 0;
        status = tt_term_link_uri(term, (uint32_t)event->id, uri, sizeof uri, &uri_len);
        if (status == TT_FULL || status == TT_EMPTY)
            status = TT_OK;
    }
    free(text);
    free(title);
    return status;
}

static tt_status poll_until_done(struct shared *shared) {
    tt_event event;
    unsigned long events = 0, reads = 0;
    for (;;) {
        int done = atomic_load(&shared->feeding_done);
        memset(&event, 0, sizeof event);
        event.struct_size = sizeof event;
        tt_status status;
        while ((status = tt_term_poll_event(shared->term, &event)) == TT_OK) {
            CHECK(handle(shared->term, &event, &reads));
            events++;
            event.struct_size = sizeof event;
        }
        if (status != TT_EMPTY)
            return status;
        if (done)
            break;
    }
    printf("%lu events polled, %lu clipboard reads answered\n", events, reads);
    return TT_OK;
}

static int same(const uint8_t *text, size_t len, const char *want) {
    return len == strlen(want) && (len == 0 || memcmp(text, want, len) == 0);
}

/* Polls one event of `kind` whose text is `body`. */
static tt_status expect(tt_term *term, uint32_t kind, const char *body, tt_event *event) {
    memset(event, 0, sizeof *event);
    event->struct_size = sizeof *event;
    CHECK(tt_term_poll_event(term, event));
    uint8_t *text = NULL;
    size_t len = 0;
    CHECK(event_text(term, event, TT_EVENT_TEXT_BODY, &text, &len));
    int ok = event->kind == kind && same(text, len, body);
    if (!ok)
        fprintf(stderr, "event %u with %.*s, want %u with %s\n", event->kind,
                (int)len, (const char *)text, kind, body);
    free(text);
    return ok ? TT_OK : TT_INVALID;
}

static tt_status each_kind(tt_term *term) {
    tt_event event;
    CHECK(feed(term, "\x1b]1;icon\a\x1b]7;file://h/tmp\a\x1b]133;A\a"
                     "\x1b]52;s;aGk=\a\x1b]52;c;?\a\x1b]8;;http://y\x1b\\L\x1b]8;;\x1b\\"
                     "\x1b]9;hello\a"));
    CHECK(expect(term, TT_EVENT_TITLE, "icon", &event));
    if (event.detail != TT_TITLE_ICON)
        return TT_INVALID;
    CHECK(expect(term, TT_EVENT_WORKING_DIRECTORY, "file://h/tmp", &event));
    CHECK(expect(term, TT_EVENT_SHELL_MARK, "", &event));
    if (event.detail != 'A')
        return TT_INVALID;
    CHECK(expect(term, TT_EVENT_CLIPBOARD_WRITE, "hi", &event));
    if (event.detail != 's')
        return TT_INVALID;
    CHECK(expect(term, TT_EVENT_CLIPBOARD_READ, "", &event));
    CHECK(tt_term_clipboard_deny(term, event.id));
    if (tt_term_clipboard_deny(term, event.id) != TT_INVALID)
        return TT_INVALID;
    CHECK(expect(term, TT_EVENT_LINK, "http://y", &event));
    uint64_t serial = event.serial;
    uint8_t uri[16];
    size_t uri_len = 0;
    CHECK(tt_term_link_uri(term, (uint32_t)event.id, uri, sizeof uri, &uri_len));
    if (!same(uri, uri_len, "http://y"))
        return TT_INVALID;
    CHECK(expect(term, TT_EVENT_NOTIFICATION, "hello", &event));
    if (event.title_len != 0)
        return TT_INVALID;
    /* Only the event polled last keeps its text. */
    size_t len = 0;
    if (tt_term_event_text(term, serial, TT_EVENT_TEXT_BODY, NULL, 0, &len) != TT_EMPTY)
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
    options.scrollback = 100;
    struct shared shared = {.term = NULL, .feeder_status = TT_OK};
    atomic_init(&shared.feeding_done, 0);
    if (tt_term_new(&options, &shared.term) != TT_OK)
        return 1;

    pthread_t thread;
    if (pthread_create(&thread, NULL, feeder, &shared) != 0)
        return 1;
    tt_status polled = poll_until_done(&shared);
    pthread_join(thread, NULL);
    tt_status kinds = polled == TT_OK ? each_kind(shared.term) : polled;
    tt_term_free(shared.term);

    if (shared.feeder_status != TT_OK || polled != TT_OK || kinds != TT_OK) {
        fprintf(stderr, "feeder %d, poller %d, kinds %d\n",
                (int)shared.feeder_status, (int)polled, (int)kinds);
        return 1;
    }
    puts("ok");
    return 0;
}
