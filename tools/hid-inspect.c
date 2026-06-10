#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <setupapi.h>
#include <hidsdi.h>
#include <hidpi.h>

static void hexdump(const unsigned char *buf, int len) {
    for (int i = 0; i < len; i++) {
        printf("%02x ", buf[i]);
        if ((i + 1) % 16 == 0) printf("\n");
    }
    if (len % 16 != 0) printf("\n");
}

static void dump_button_caps(int count, HIDP_BUTTON_CAPS *caps) {
    for (int i = 0; i < count; i++) {
        HIDP_BUTTON_CAPS *c = &caps[i];
        printf("    [%d] UsagePage=0x%04x Usage=%s {min=%d, max=%d} "
               "ReportID=%d IsAlias=%d LinkCollection=%d DataIndex=%d/%d\n",
               i, c->UsagePage,
               c->IsRange ? "range" : "single",
               c->IsRange ? c->Range.UsageMin : c->NotRange.Usage,
               c->IsRange ? c->Range.UsageMax : c->NotRange.Usage,
               c->ReportID, c->IsAlias,
               c->LinkCollection,
               c->IsRange ? c->Range.DataIndexMin : c->NotRange.DataIndex,
               c->IsRange ? c->Range.DataIndexMax : c->NotRange.DataIndex);
    }
}

static void dump_value_caps(int count, HIDP_VALUE_CAPS *caps) {
    for (int i = 0; i < count; i++) {
        HIDP_VALUE_CAPS *c = &caps[i];
        printf("    [%d] UsagePage=0x%04x Usage=single {min=%d} "
               "ReportID=%d BitSize=%d Logical=(%d,%d)\n",
               i, c->UsagePage,
               c->IsRange ? c->Range.UsageMin : c->NotRange.Usage,
               c->ReportID, c->BitSize,
               c->LogicalMin, c->LogicalMax);
    }
}

static void inspect_device(HANDLE h) {
    HIDD_ATTRIBUTES attr = { .Size = sizeof(attr) };
    if (!HidD_GetAttributes(h, &attr)) {
        printf("  HidD_GetAttributes FAILED: %lu\n", GetLastError());
        return;
    }
    printf("  Attributes: VID=%04x PID=%04x Version=%04x\n",
           attr.VendorID, attr.ProductID, attr.VersionNumber);

    WCHAR product[128] = {0};
    if (HidD_GetProductString(h, product, sizeof(product)))
        printf("  Product: %ls\n", product);

    PHIDP_PREPARSED_DATA ppd = NULL;
    if (!HidD_GetPreparsedData(h, &ppd)) {
        printf("  HidD_GetPreparsedData FAILED: %lu\n", GetLastError());
        return;
    }

    // --- HidP_GetCaps ---
    HIDP_CAPS caps = {0};
    if (HidP_GetCaps(ppd, &caps) != HIDP_STATUS_SUCCESS) {
        printf("  HidP_GetCaps FAILED\n");
        HidD_FreePreparsedData(ppd);
        return;
    }
    printf("\n  === Caps ===\n");
    printf("    Usage=0x%04x:0x%04x\n", caps.UsagePage, caps.Usage);
    printf("    InputReportByteLength=%d OutputReportByteLength=%d FeatureReportByteLength=%d\n",
           caps.InputReportByteLength, caps.OutputReportByteLength, caps.FeatureReportByteLength);
    printf("    LinkCollectionNodes=%d\n", caps.NumberLinkCollectionNodes);
    printf("    InputButtonCaps=%d InputValueCaps=%d\n",
           caps.NumberInputButtonCaps, caps.NumberInputValueCaps);
    printf("    OutputButtonCaps=%d OutputValueCaps=%d\n",
           caps.NumberOutputButtonCaps, caps.NumberOutputValueCaps);
    printf("    FeatureButtonCaps=%d FeatureValueCaps=%d\n",
           caps.NumberFeatureButtonCaps, caps.NumberFeatureValueCaps);

    // --- deepInspect: game's exact queries ---
    printf("\n  === DeepInspect (game queries) ===\n");

    // 1. HidP_GetLinkCollectionNodes
    if (caps.NumberLinkCollectionNodes > 0) {
        HIDP_LINK_COLLECTION_NODE *nodes =
            calloc(caps.NumberLinkCollectionNodes, sizeof(*nodes));
        ULONG n = caps.NumberLinkCollectionNodes;
        NTSTATUS s = HidP_GetLinkCollectionNodes(nodes, &n, ppd);
        printf("  LinkCollectionNodes: ntstatus=0x%lx count=%lu\n", s, n);
        for (ULONG i = 0; i < n && i < 10; i++)
            printf("    [%lu] Page=0x%04x Usage=0x%04x Parent=%d Children=%d\n",
                   i, nodes[i].LinkUsagePage, nodes[i].LinkUsage,
                   nodes[i].Parent, nodes[i].NumberOfChildren);
        free(nodes);
    }

    // 2. HidP_GetSpecificButtonCaps(OUTPUT, page=0x0F, usage=0x9A)
    {
        USHORT bc = 0;
        NTSTATUS s = HidP_GetSpecificButtonCaps(HidP_Output, 0x0F, 0, 0x9A,
            NULL, &bc, ppd);
        HIDP_BUTTON_CAPS *bcaps = bc ? calloc(bc, sizeof(*bcaps)) : NULL;
        if (bcaps) {
            s = HidP_GetSpecificButtonCaps(HidP_Output, 0x0F, 0, 0x9A,
                bcaps, &bc, ppd);
            printf("  SpecificButtonCaps(OUTPUT, page=0x0F, usage=0x9A): ntstatus=0x%lx count=%d\n", s, bc);
            dump_button_caps(bc, bcaps);
            free(bcaps);
        } else {
            printf("  SpecificButtonCaps(OUTPUT, page=0x0F, usage=0x9A): ntstatus=0x%lx count=0\n", s);
        }
    }

    // 3. HidP_GetSpecificButtonCaps(INPUT, page=Button=0x09, usage=0)
    {
        USHORT bc = 0;
        NTSTATUS s = HidP_GetSpecificButtonCaps(HidP_Input, 0x09, 0, 0,
            NULL, &bc, ppd);
        HIDP_BUTTON_CAPS *bcaps = bc ? calloc(bc, sizeof(*bcaps)) : NULL;
        if (bcaps) {
            s = HidP_GetSpecificButtonCaps(HidP_Input, 0x09, 0, 0,
                bcaps, &bc, ppd);
            printf("  SpecificButtonCaps(INPUT, page=Button, usage=0): ntstatus=0x%lx count=%d\n", s, bc);
            dump_button_caps(bc, bcaps);
            free(bcaps);
        } else {
            printf("  SpecificButtonCaps(INPUT, page=Button, usage=0): ntstatus=0x%lx count=0\n", s);
        }
    }

    // 4. HidP_GetSpecificValueCaps(INPUT, page=0x01, usage=0x30) — X axis
    {
        USHORT vc = 0;
        NTSTATUS s = HidP_GetSpecificValueCaps(HidP_Input, 0x01, 0, 0x30,
            NULL, &vc, ppd);
        HIDP_VALUE_CAPS *vcaps = vc ? calloc(vc, sizeof(*vcaps)) : NULL;
        if (vcaps) {
            s = HidP_GetSpecificValueCaps(HidP_Input, 0x01, 0, 0x30,
                vcaps, &vc, ppd);
            printf("  SpecificValueCaps(INPUT, page=0x01, usage=0x30): ntstatus=0x%lx count=%d\n", s, vc);
            dump_value_caps(vc, vcaps);
            free(vcaps);
        } else {
            printf("  SpecificValueCaps(INPUT, page=0x01, usage=0x30): ntstatus=0x%lx count=0\n", s);
        }
    }

    // 5. HidP_GetSpecificValueCaps(INPUT, page=0x01, usage=0x31) — Y axis
    {
        USHORT vc = 0;
        NTSTATUS s = HidP_GetSpecificValueCaps(HidP_Input, 0x01, 0, 0x31,
            NULL, &vc, ppd);
        HIDP_VALUE_CAPS *vcaps = vc ? calloc(vc, sizeof(*vcaps)) : NULL;
        if (vcaps) {
            s = HidP_GetSpecificValueCaps(HidP_Input, 0x01, 0, 0x31,
                vcaps, &vc, ppd);
            printf("  SpecificValueCaps(INPUT, page=0x01, usage=0x31): ntstatus=0x%lx count=%d\n", s, vc);
            dump_value_caps(vc, vcaps);
            free(vcaps);
        } else {
            printf("  SpecificValueCaps(INPUT, page=0x01, usage=0x31): ntstatus=0x%lx count=0\n", s);
        }
    }

    // 6. HidP_GetSpecificValueCaps(INPUT, page=0x02, usage=0xC8)
    {
        USHORT vc = 0;
        NTSTATUS s = HidP_GetSpecificValueCaps(HidP_Input, 0x02, 0, 0xC8,
            NULL, &vc, ppd);
        HIDP_VALUE_CAPS *vcaps = vc ? calloc(vc, sizeof(*vcaps)) : NULL;
        if (vcaps) {
            s = HidP_GetSpecificValueCaps(HidP_Input, 0x02, 0, 0xC8,
                vcaps, &vc, ppd);
            printf("  SpecificValueCaps(INPUT, page=0x02, usage=0xC8): ntstatus=0x%lx count=%d\n", s, vc);
            dump_value_caps(vc, vcaps);
            free(vcaps);
        } else {
            printf("  SpecificValueCaps(INPUT, page=0x02, usage=0xC8): ntstatus=0x%lx count=0\n", s);
        }
    }

    HidD_FreePreparsedData(ppd);

    // --- Feature reports ---
    printf("\n  === Feature Reports ===\n");
    { unsigned char buf[64] = {0}; buf[0] = 0x12;
      if (HidD_GetFeature(h, buf, 16))
        { printf("    GetFeature(0x12) OK: "); hexdump(buf, 16); }
      else printf("    GetFeature(0x12) FAILED: %lu\n", GetLastError()); }
    { unsigned char buf[64] = {0}; buf[0] = 0xA3;
      if (HidD_GetFeature(h, buf, 49))
        { printf("    GetFeature(0xA3) OK: "); hexdump(buf, 49); }
      else printf("    GetFeature(0xA3) FAILED: %lu\n", GetLastError()); }
    { unsigned char buf[64] = {0}; buf[0] = 0x02;
      if (HidD_GetFeature(h, buf, 37))
        { printf("    GetFeature(0x02) OK: "); hexdump(buf, 37); }
      else printf("    GetFeature(0x02) FAILED: %lu\n", GetLastError()); }
    { unsigned char buf[64] = {0}; buf[0] = 0x14;
      if (HidD_SetFeature(h, buf, 17))
        { printf("    SetFeature(0x14) OK: "); hexdump(buf, 17); }
      else printf("    SetFeature(0x14) FAILED: %lu\n", GetLastError()); }
}

int main(int argc, char **argv) {
    GUID hid_guid;
    HidD_GetHidGuid(&hid_guid);

    int target_pid = argc > 1 ? (int)strtoul(argv[1], NULL, 16) : 0;

    HDEVINFO devs = SetupDiGetClassDevsW(&hid_guid, NULL, NULL,
        DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
    if (devs == INVALID_HANDLE_VALUE) {
        printf("SetupDiGetClassDevs failed: %lu\n", GetLastError());
        return 1;
    }

    SP_DEVICE_INTERFACE_DATA iface = { .cbSize = sizeof(iface) };
    DWORD idx = 0;
    int found = 0;

    while (SetupDiEnumDeviceInterfaces(devs, NULL, &hid_guid, idx++, &iface)) {
        DWORD needed;
        SetupDiGetDeviceInterfaceDetailW(devs, &iface, NULL, 0, &needed, NULL);
        PSP_DEVICE_INTERFACE_DETAIL_DATA_W detail = malloc(needed);
        detail->cbSize = sizeof(*detail);
        if (!SetupDiGetDeviceInterfaceDetailW(devs, &iface, detail, needed, NULL, NULL)) {
            free(detail); continue;
        }

        HANDLE h = CreateFileW(detail->DevicePath, GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL);
        if (h == INVALID_HANDLE_VALUE) { free(detail); continue; }

        HIDD_ATTRIBUTES attr = { .Size = sizeof(attr) };
        if (!HidD_GetAttributes(h, &attr)) { CloseHandle(h); free(detail); continue; }
        if (attr.VendorID != 0x054C) { CloseHandle(h); free(detail); continue; }
        if (target_pid && attr.ProductID != target_pid) { CloseHandle(h); free(detail); continue; }

        printf("\n========================================\n");
        printf("=== VID=%04x PID=%04x ===\n", attr.VendorID, attr.ProductID);
        printf("========================================\n");
        inspect_device(h);
        CloseHandle(h);
        free(detail);
        found++;
    }

    SetupDiDestroyDeviceInfoList(devs);
    if (!found) printf("No Sony devices found.\n");
    return 0;
}
