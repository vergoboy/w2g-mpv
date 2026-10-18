/* w2g: in-process sync plugin for mpv (loaded with --script=w2g.so).
 * Applies the room state to the player; mpv core is untouched.
 * Input : script-message-to w2g w2g-state <seq> <playing> <anchor_pos> <anchor_server_time> <rate> [waiting]
 *         (transport/clock come later; for now server time == CLOCK_MONOTONIC of this host)
 * Output: print-text lines "w2g-emit <event>" (transport will replace this).
 * Env   : W2G_ROLE=controller|watcher (default controller), W2G_MODE=room|local (default room) */
#include <mpv/client.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "w2g_sync.h"

static double mono(void) {
    struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static void say(mpv_handle *h, const char *s) {
    const char *cmd[] = {"print-text", s, NULL};
    mpv_command(h, cmd);
}

int mpv_open_cplugin(mpv_handle *h) {
    const char *role_s = getenv("W2G_ROLE"), *mode_s = getenv("W2G_MODE");
    int controller = !(role_s && !strcmp(role_s, "watcher"));
    int local = mode_s && !strcmp(mode_s, "local");
    W2gClient *c = w2g_client_new(local);
    w2g_client_on_connected(c);

    mpv_observe_property(h, 1, "pause", MPV_FORMAT_FLAG);
    mpv_observe_property(h, 2, "paused-for-cache", MPV_FORMAT_FLAG);

    int pause_pending = 0, pause_expected = 0, seek_flag = 0;
    double cur_speed = 1.0;
    char buf[160];

    for (;;) {
        mpv_event *ev = mpv_wait_event(h, 0.2);
        if (ev->event_id == MPV_EVENT_SHUTDOWN) break;

        if (ev->event_id == MPV_EVENT_CLIENT_MESSAGE) {
            mpv_event_client_message *m = ev->data;
            if (m->num_args >= 6 && !strcmp(m->args[0], "w2g-state")) {
                W2gState s = {
                    .seq = strtoull(m->args[1], NULL, 10), .playing = atoi(m->args[2]),
                    .anchor_pos = atof(m->args[3]), .anchor_server_time = atof(m->args[4]),
                    .rate = atof(m->args[5]), .waiting = m->num_args > 6 ? atoi(m->args[6]) : 0 };
                if (w2g_client_on_state(c, &s)) {
                    /* apply the STORED room, never the incoming message: a duplicate seq must not change anything */
                    int pl = 0; double e = w2g_client_expected(c, mono(), &pl);
                    int cur = 0; mpv_get_property(h, "pause", MPV_FORMAT_FLAG, &cur);
                    int want = !pl;
                    if (e >= 0 && cur != want) { pause_pending = 1; pause_expected = want;
                        mpv_set_property(h, "pause", MPV_FORMAT_FLAG, &want); }
                }
            } else if (m->num_args >= 2 && !strcmp(m->args[0], "w2g-reconnect")) {
                w2g_client_on_reconnect(c);
            }
        } else if (ev->event_id == MPV_EVENT_PROPERTY_CHANGE) {
            mpv_event_property *p = ev->data;
            if (p->format == MPV_FORMAT_FLAG && !strcmp(p->name, "pause")) {
                int v = *(int *)p->data;
                if (pause_pending && v == pause_expected) pause_pending = 0;   /* echo of our own change */
                else if (w2g_client_can_emit(c, controller)) say(h, v ? "w2g-emit pause" : "w2g-emit play");
                else if (!local && !controller) {          /* watcher: local pause goes back to the room */
                    int pl = 0; double e = w2g_client_expected(c, mono(), &pl);
                    int want = !pl;
                    if (e >= 0 && v != want) { pause_pending = 1; pause_expected = want;
                        mpv_set_property(h, "pause", MPV_FORMAT_FLAG, &want); }
                }
            } else if (p->format == MPV_FORMAT_FLAG && !strcmp(p->name, "paused-for-cache")) {
                w2g_client_set_buffering(c, *(int *)p->data);   /* informational only */
            }
        } else if (ev->event_id == MPV_EVENT_SEEK) {
            seek_flag = 1;
        } else if (ev->event_id == MPV_EVENT_PLAYBACK_RESTART && seek_flag) {
            seek_flag = 0;
            double pos;
            if (mpv_get_property(h, "time-pos", MPV_FORMAT_DOUBLE, &pos) >= 0) {
                int k = w2g_client_player_seek(c, pos);
                if (k == 2 && w2g_client_can_emit(c, controller)) {
                    snprintf(buf, sizeof buf, "w2g-emit seek %.3f", pos); say(h, buf);
                }
            }
        }

        /* drift tick (runs on every wake-up, at least every 200 ms) */
        double pos;
        if (mpv_get_property(h, "time-pos", MPV_FORMAT_DOUBLE, &pos) >= 0) {
            double now = mono();
            W2gDrift d;
            if (w2g_client_drift(c, pos, now, now, &d)) {
                if (d.kind == 3) {
                    snprintf(buf, sizeof buf, "%.3f", d.value);
                    const char *cmd[] = {"seek", buf, "absolute+exact", NULL};
                    mpv_command(h, cmd);
                } else if (d.kind == 2 || d.kind == 1) {
                    double want = d.kind == 2 ? d.value : w2g_client_room_rate(c);  /* ok -> room's own rate */
                    if (want != cur_speed) { mpv_set_property(h, "speed", MPV_FORMAT_DOUBLE, &want); cur_speed = want; }
                }
            }
        }
    }
    w2g_client_free(c);
    return 0;
}
