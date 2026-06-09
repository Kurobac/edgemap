#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <linux/hidraw.h>

static void hexdump(const unsigned char *buf, int len) {
    for (int i = 0; i < len; i++) {
        printf("%02x ", buf[i]);
        if ((i + 1) % 16 == 0) printf("\n");
    }
    if (len % 16 != 0) printf("\n");
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s /dev/hidrawN\n", argv[0]);
        return 1;
    }
    int fd = open(argv[1], O_RDWR);
    if (fd < 0) { perror("open"); return 1; }
    printf("=== %s ===\n\n", argv[1]);

    { unsigned char buf[64] = {0}; buf[0] = 0x12;
      int r = ioctl(fd, HIDIOCGFEATURE(16), buf);
      printf("GetFeature(0x12): %s ", r < 0 ? "FAIL" : "OK  ");
      if (r >= 0) hexdump(buf, 16); else perror(""); }

    { unsigned char buf[64] = {0}; buf[0] = 0xA3;
      int r = ioctl(fd, HIDIOCGFEATURE(49), buf);
      printf("GetFeature(0xA3): %s ", r < 0 ? "FAIL" : "OK  ");
      if (r >= 0) hexdump(buf, 49); else perror(""); }

    { unsigned char buf[64] = {0}; buf[0] = 0x14;
      int r = ioctl(fd, HIDIOCSFEATURE(17), buf);
      printf("SetFeature(0x14): %s ", r < 0 ? "FAIL" : "OK  ");
      if (r >= 0) hexdump(buf, 17); else perror(""); }

    { unsigned char buf[64] = {0}; buf[0] = 0x02;
      int r = ioctl(fd, HIDIOCGFEATURE(37), buf);
      printf("GetFeature(0x02): %s ", r < 0 ? "FAIL" : "OK  ");
      if (r >= 0) hexdump(buf, 37); else perror(""); }

    close(fd);
    return 0;
}
