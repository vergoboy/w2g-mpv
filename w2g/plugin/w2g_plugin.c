/* w2g: in-process sync plugin for mpv (loaded with --script=w2g.so). mpv core is untouched.
 *
 * Room state sources:
 *   - network : set W2G_URL (ws://host:port), W2G_TOKEN, W2G_ROOM; state + clock come from w2g-server.
 *   - dev/test: otherwise `script-message-to w2g w2g-state <seq> <playing> <pos> <server_time> <rate>`
 *               with server time == CLOCK_MONOTONIC of this host.
 * Local events (user pause/play/seek) are sent to the server (network mode) and printed as
 * "w2g-emit ..." lines.  Env: W2G_ROLE=controller|watcher (client-side gate; the server enforces roles),
 * W2G_MODE=room|local. */
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

typedef struct {
    mpv_handle *h;
    W2gClient *c;
    W2gNet *net;
    int controller, local;
    int pause_pending, pause_expected, seek_flag;
    double cur_speed;
    double hold_until;       /* no drift correction until the room answers our own command */
    unsigned long long last_seq;
} P;

static void say(P *p, const char *s) { const char *cmd[] = {"print-text", s, NULL}; mpv_command(p->h, cmd); }
static double server_now(P *p) { return p->net ? w2g_net_server_now(p->net) : mono(); }

static void send_net(P *p, int cmd, double a) { if (p->net) w2g_net_send(p->net, cmd, a, 0, 0); }
static void hold(P *p) { p->hold_until = mono() + 1.0; }

static void apply_stored_pause(P *p) {
    /* apply the STORED room, never an incoming message: a duplicate seq must not change anything */
    int pl = 0; double e = w2g_client_expected(p->c, server_now(p) < 0 ? 0 : server_now(p), &pl);
    int cur = 0; mpv_get_property(p->h, "pause", MPV_FORMAT_FLAG, &cur);
    int want = !pl;
    if (e >= 0 && cur != want) {
        p->pause_pending = 1; p->pause_expected = want;
        mpv_set_property(p->h, "pause", MPV_FORMAT_FLAG, &want);
    }
}

static void on_room_state(P *p, const W2gState *s) {
    if (s->seq > p->last_seq) { p->last_seq = s->seq; p->hold_until = 0; }   /* the room answered */
    if (w2g_client_on_state(p->c, s)) apply_stored_pause(p);
}

int mpv_open_cplugin(mpv_handle *h) {
    P p = {.h = h, .cur_speed = 1.0};
    const char *role_s = getenv("W2G_ROLE"), *mode_s = getenv("W2G_MODE");
    const char *url = getenv("W2G_URL"), *tok = getenv("W2G_TOKEN"), *room = getenv("W2G_ROOM");
    p.controller = !(role_s && !strcmp(role_s, "watcher"));
    p.local = mode_s && !strcmp(mode_s, "local");
    p.c = w2g_client_new(p.local);
    w2g_client_on_connected(p.c);
    if (url && !p.local) p.net = w2g_net_start(url, tok ? tok : "", room ? room : "default");

    mpv_observe_property(h, 1, "pause", MPV_FORMAT_FLAG);
    mpv_observe_property(h, 2, "paused-for-cache", MPV_FORMAT_FLAG);
    char buf[160];

    for (;;) {
        mpv_event *ev = mpv_wait_event(h, p.net ? 0.05 : 0.2);
        if (ev->event_id == MPV_EVENT_SHUTDOWN) break;

        if (p.net) {
            if (w2g_net_poll_reconnect(p.net)) w2g_client_on_reconnect(p.c);
            W2gState st;
            while (w2g_net_poll_state(p.net, &st)) on_room_state(&p, &st);
        }

        if (ev->event_id == MPV_EVENT_CLIENT_MESSAGE) {
            mpv_event_client_message *m = ev->data;
            if (m->num_args >= 6 && !strcmp(m->args[0], "w2g-state")) {
                W2gState s = {
                    .seq = strtoull(m->args[1], NULL, 10), .playing = atoi(m->args[2]),
                    .anchor_pos = atof(m->args[3]), .anchor_server_time = atof(m->args[4]),
                    .rate = atof(m->args[5]), .waiting = m->num_args > 6 ? atoi(m->args[6]) : 0 };
                on_room_state(&p, &s);
            } else if (m->num_args >= 1 && !strcmp(m->args[0], "w2g-reconnect")) {
                w2g_client_on_reconnect(p.c);
            }
        } else if (ev->event_id == MPV_EVENT_PROPERTY_CHANGE) {
            mpv_event_property *pr = ev->data;
            if (pr->format == MPV_FORMAT_FLAG && !strcmp(pr->name, "pause")) {
                int v = *(int *)pr->data;
                if (p.pause_pending && v == p.pause_expected) p.pause_pending = 0;   /* echo of our own change */
                else if (w2g_client_can_emit(p.c, p.controller)) {
                    say(&p, v ? "w2g-emit pause" : "w2g-emit play");
                    send_net(&p, v ? 1 : 0, 0); hold(&p);
                } else if (!p.local && !p.controller) {   /* watcher: local pause goes back to the room */
                    int pl = 0; double e = w2g_client_expected(p.c, 0, &pl);
                    int want = !pl;
                    if (e >= 0 && v != want) { p.pause_pending = 1; p.pause_expected = want;
                        mpv_set_property(h, "pause", MPV_FORMAT_FLAG, &want); }
                }
            } else if (pr->format == MPV_FORMAT_FLAG && !strcmp(pr->name, "paused-for-cache")) {
                w2g_client_set_buffering(p.c, *(int *)pr->data);   /* informational only */
            }
        } else if (ev->event_id == MPV_EVENT_SEEK) {
            p.seek_flag = 1;
        } else if (ev->event_id == MPV_EVENT_PLAYBACK_RESTART && p.seek_flag) {
            p.seek_flag = 0;
            double pos;
            if (mpv_get_property(h, "time-pos", MPV_FORMAT_DOUBLE, &pos) >= 0) {
                int k = w2g_client_player_seek(p.c, pos);
                if (k == 2 && w2g_client_can_emit(p.c, p.controller)) {
                    snprintf(buf, sizeof buf, "w2g-emit seek %.3f", pos); say(&p, buf);
                    send_net(&p, 2, pos); hold(&p);
                }
            }
        }

        /* drift tick (every wake-up) */
        double pos, sn = server_now(&p);
        int held = p.seek_flag || mono() < p.hold_until;   /* a seek is in flight, or we await the room */
        if (!held && sn >= 0 && mpv_get_property(h, "time-pos", MPV_FORMAT_DOUBLE, &pos) >= 0) {
            W2gDrift d;
            if (w2g_client_drift(p.c, pos, sn, mono(), &d)) {
                if (d.kind == 3) {
                    snprintf(buf, sizeof buf, "%.3f", d.value);
                    const char *cmd[] = {"seek", buf, "absolute+exact", NULL};
                    mpv_command(h, cmd);
                } else if (d.kind == 2 || d.kind == 1) {
                    double want = d.kind == 2 ? d.value : w2g_client_room_rate(p.c);
                    if (want != p.cur_speed) { mpv_set_property(h, "speed", MPV_FORMAT_DOUBLE, &want); p.cur_speed = want; }
                }
            }
        }
    }
    if (p.net) w2g_net_free(p.net);
    w2g_client_free(p.c);
    return 0;
}
