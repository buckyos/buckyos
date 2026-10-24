#include "../../src/kernel/buckyos_agent_daemon/src/computer/native.m"

int main(void) {
    BuckyDesktop desktop;
    bucky_desktop_probe(&desktop);
    if (!desktop.ready || !desktop.capture) {
        printf("capture unavailable: session=%u capture=%u\n", desktop.ready, desktop.capture);
        return 77;
    }
    unsigned char *bytes = NULL;
    size_t length = 0;
    int code = bucky_desktop_capture(desktop.display, desktop.width, desktop.height,
                                     10000, &bytes, &length);
    if (code || !bytes || length < 24 || memcmp(bytes, "\x89PNG\r\n\x1a\n", 8)) {
        fprintf(stderr, "capture failed: code=%d length=%zu\n", code, length);
        free(bytes);
        return 1;
    }
    uint32_t width = ((uint32_t)bytes[16] << 24) | ((uint32_t)bytes[17] << 16)
        | ((uint32_t)bytes[18] << 8) | bytes[19];
    uint32_t height = ((uint32_t)bytes[20] << 24) | ((uint32_t)bytes[21] << 16)
        | ((uint32_t)bytes[22] << 8) | bytes[23];
    free(bytes);
    if (width != desktop.width || height != desktop.height) return 1;
    printf("ScreenCaptureKit PNG: %ux%u, %zu bytes; pixels discarded, no input dispatched\n",
           width, height, length);
    return 0;
}
