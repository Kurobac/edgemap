#define _GNU_SOURCE
#include <dlfcn.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <libudev.h>

const char *udev_device_get_sysattr_value(struct udev_device *dev, const char *attr)
{
    static const char *(*real)(struct udev_device *, const char *);
    static char buf[4096];
    static int calls;

    if (!real)
        real = dlsym(RTLD_NEXT, "udev_device_get_sysattr_value");

    const char *ret = real(dev, attr);
    if (!ret) return ret;

    if (!strcmp(attr, "uevent")) {
        if (++calls <= 5 && strstr(ret, "HID_ID="))
            fprintf(stderr, "[version-fix] uevent call #%d, has_PRODUCT=%d\n",
                    calls, strstr(ret, "PRODUCT=") != NULL);

        if (strstr(ret, "HID_ID=0003:0000054C:000005C4") &&
            !strstr(ret, "PRODUCT="))
        {
            int len = snprintf(buf, sizeof(buf),
                "%sPRODUCT=3/54c/5c4/100\n", ret);
            fprintf(stderr, "[version-fix] INJECTED version, len=%d\n", len);
            if (len > 0 && len < (int)sizeof(buf))
                return buf;
        }
    }

    return ret;
}
