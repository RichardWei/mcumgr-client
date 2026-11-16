#include "mcumgr.h"
#include <stdio.h>
#include <string.h>

int main(void) {
    McConnection *connection = NULL;
    McBuffer result = {NULL, 0};
    if (mc_reset(NULL) != MC_INVALID_ARGUMENT) return 1;
    if (mc_last_error(&result) != MC_OK || result.len == 0) return 2;
    if (mc_buffer_free(&result) != MC_OK) return 3;
    /* The reserved "test" device is upstream's serial simulator. No hardware is used. */
    if (mc_open_serial("test", 115200, 60, 128, 512, &connection) != MC_OK) return 4;
    if (mc_image_list_json(connection, &result) != MC_OK) return 5;
    if (fwrite(result.data, 1, result.len, stdout) != result.len) return 6;
    if (result.len == 0) return 7;
    if (mc_buffer_free(&result) != MC_OK) return 8;
    if (mc_close(&connection) != MC_OK || connection != NULL) return 9;
    return 0;
}
