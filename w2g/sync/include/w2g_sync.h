#ifndef W2G_SYNC_H
#define W2G_SYNC_H
#include <stdint.h>
typedef struct W2gClient W2gClient;
typedef struct { uint64_t seq; int32_t playing, waiting; double anchor_pos, anchor_server_time, rate; } W2gState;
typedef struct { int32_t kind; double value; } W2gDrift; /* 0 hold 1 ok 2 rate 3 seek */
W2gClient *w2g_client_new(int32_t local_mode);
void w2g_client_free(W2gClient *);
void w2g_client_on_connected(W2gClient *);
void w2g_client_on_reconnect(W2gClient *);
int32_t w2g_client_on_state(W2gClient *, const W2gState *);
void w2g_client_set_buffering(W2gClient *, int32_t);
int32_t w2g_client_drift(W2gClient *, double local_pos, double now_server, double now_local, W2gDrift *out);
int32_t w2g_client_player_seek(W2gClient *, double target); /* 0 echo 1 own 2 user */
int32_t w2g_client_can_emit(W2gClient *, int32_t role_controller);
double w2g_client_expected(W2gClient *, double now_server, int32_t *out_playing);
double w2g_client_room_rate(W2gClient *);
#endif
