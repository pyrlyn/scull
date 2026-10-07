/*
 * Copyright (c) 2026 Ivan Tugay
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The configuration handle as a host uses it, from two threads: one edits
 * the file through tt_config_set and by hand, the other polls the view and
 * reads every string and key binding it points to. Built and run under the
 * sanitizers by `just c-abi-test`.
 */

#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "scull.h"

enum {
    SETS = 40,
    /* Most polls before giving up on a change; each waits 10 ms. */
    MAX_POLLS = 3000,
    MIN_SIZE = 8,
    FINAL_SIZE = 19,
};

static atomic_int wakeups;
static atomic_int editing_done;

static void wake(void *userdata) {
    (void)userdata;
    atomic_fetch_add(&wakeups, 1);
}

static tt_str str(const char *s) {
    tt_str out = {(const uint8_t *)s, strlen(s)};
    return out;
}

static void sleep_ms(long ms) {
    struct timespec ts = {0, ms * 1000000L};
    nanosleep(&ts, NULL);
}

static void *editor(void *arg) {
    tt_config *config = arg;
    char value[16];
    for (int i = 0; i < SETS; i++) {
        /* Sizes within the accepted range, ending on the one polled for. */
        snprintf(value, sizeof value, "%d", i == SETS - 1 ? FINAL_SIZE : MIN_SIZE + i % 5);
        if (tt_config_set(config, str("font.size"), str(value)) != TT_OK) {
            fprintf(stderr, "tt_config_set refused %s\n", value);
            exit(1);
        }
        sleep_ms(2);
    }
    atomic_store(&editing_done, 1);
    return NULL;
}

int main(void) {
    char dir[] = "/tmp/scull-config-XXXXXX";
    if (!mkdtemp(dir)) {
        perror("mkdtemp");
        return 1;
    }
    char path[64];
    snprintf(path, sizeof path, "%s/config.toml", dir);
    FILE *file = fopen(path, "w");
    if (!file) {
        perror("fopen");
        return 1;
    }
    fputs("scrollback = 5\n", file);
    fclose(file);

    tt_config_options options;
    memset(&options, 0, sizeof options);
    options.struct_size = sizeof options;
    options.abi_version = TT_ABI_VERSION;
    options.path = str(path);
    options.wakeup = wake;
    tt_config *config = NULL;
    if (tt_config_new(&options, &config) != TT_OK) {
        fprintf(stderr, "tt_config_new failed\n");
        return 1;
    }

    pthread_t thread;
    pthread_create(&thread, NULL, editor, config);
    int status = 0;
    float size = 0;
    for (int i = 0; i < MAX_POLLS; i++) {
        tt_config_view view;
        memset(&view, 0, sizeof view);
        view.struct_size = sizeof view;
        if (tt_config_poll(config, &view) != TT_OK) {
            fprintf(stderr, "tt_config_poll failed\n");
            status = 1;
            break;
        }
        /* Touch every byte the view points to, as a host would. */
        unsigned sum = 0;
        for (size_t b = 0; b < view.font_family.len; b++) sum += view.font_family.ptr[b];
        for (size_t b = 0; b < view.path.len; b++) sum += view.path.ptr[b];
        for (size_t b = 0; b < view.error.len; b++) sum += view.error.ptr[b];
        for (size_t k = 0; k < view.keybinds_len; k++) sum += view.keybinds[k].key;
        if (view.scrollback != 5 || view.error.len != 0 || view.keybinds_len == 0 || sum == 0) {
            fprintf(stderr, "bad view: scrollback %u error %zu binds %zu\n",
                    view.scrollback, view.error.len, view.keybinds_len);
            status = 1;
            break;
        }
        size = view.font_size;
        if (atomic_load(&editing_done) && size == FINAL_SIZE) break;
        sleep_ms(10);
    }
    pthread_join(thread, NULL);
    if (status == 0 && size != FINAL_SIZE) {
        fprintf(stderr, "font size ended at %g, wanted %d\n", size, FINAL_SIZE);
        status = 1;
    }
    if (status == 0 && atomic_load(&wakeups) == 0) {
        fprintf(stderr, "no wakeup\n");
        status = 1;
    }
    tt_config_free(config);
    remove(path);
    rmdir(dir);
    return status;
}
