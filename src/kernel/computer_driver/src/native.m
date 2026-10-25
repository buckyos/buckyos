#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Carbon/Carbon.h>
#include <unistd.h>
#include <stdatomic.h>
#include <bsm/audit.h>

static atomic_bool capture_pending = false;

typedef struct {
    uint32_t uid;
    uint32_t session;
    uint32_t display;
    uint32_t width;
    uint32_t height;
    uint32_t ready;
    uint32_t capture;
    uint32_t input;
    uint32_t locked;
    int32_t focus_pid;
    uint32_t focus_on_display;
    uint32_t physical_input_down;
    uint64_t focus;
    uint64_t events;
    uint64_t hardware_events;
    uint64_t layout;
    double x;
    double y;
    double logical_width;
    double logical_height;
} BuckyDesktop;

static uint64_t event_revision(CGEventSourceStateID source) {
    CGEventType types[] = {kCGEventLeftMouseDown, kCGEventRightMouseDown,
        kCGEventOtherMouseDown, kCGEventMouseMoved, kCGEventLeftMouseDragged,
        kCGEventRightMouseDragged, kCGEventOtherMouseDragged,
        kCGEventKeyDown, kCGEventFlagsChanged, kCGEventScrollWheel};
    uint64_t revision = 0;
    for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
        revision += CGEventSourceCounterForEventType(source, types[i]);
    }
    return revision;
}

void bucky_desktop_probe(BuckyDesktop *out) {
    @autoreleasepool {
        memset(out, 0, sizeof(*out));
        NSDictionary *session = CFBridgingRelease(CGSessionCopyCurrentDictionary());
        out->uid = [session[(__bridge NSString *)kCGSessionUserIDKey] unsignedIntValue];
        auditinfo_addr_t audit = {0};
        BOOL auditReady = getaudit_addr(&audit, sizeof(audit)) == 0
            && audit.ai_asid != AU_DEFAUDITSID && audit.ai_auid == geteuid()
            && (audit.ai_flags & (AU_SESSION_FLAG_HAS_GRAPHIC_ACCESS | AU_SESSION_FLAG_HAS_CONSOLE_ACCESS))
                == (AU_SESSION_FLAG_HAS_GRAPHIC_ACCESS | AU_SESSION_FLAG_HAS_CONSOLE_ACCESS);
        out->session = audit.ai_asid;
        NSRunningApplication *front = NSWorkspace.sharedWorkspace.frontmostApplication;
        out->locked = [session[@"CGSSessionScreenIsLocked"] boolValue]
            || [front.bundleIdentifier isEqualToString:@"com.apple.loginwindow"]
            || [front.bundleIdentifier isEqualToString:@"com.apple.ScreenSaver.Engine"];
        out->ready = auditReady && session != nil && out->uid == geteuid() && out->uid != 0
            && [session[(__bridge NSString *)kCGSessionOnConsoleKey] boolValue]
            && [session[(__bridge NSString *)kCGSessionLoginDoneKey] boolValue]
            && !out->locked;
        out->display = CGMainDisplayID();
        CGRect bounds = CGDisplayBounds(out->display);
        out->x = bounds.origin.x;
        out->y = bounds.origin.y;
        out->logical_width = bounds.size.width;
        out->logical_height = bounds.size.height;
        out->width = (uint32_t)CGDisplayPixelsWide(out->display);
        out->height = (uint32_t)CGDisplayPixelsHigh(out->display);
        out->capture = CGPreflightScreenCaptureAccess();
        out->input = AXIsProcessTrusted() && CGPreflightPostEventAccess();
        out->events = event_revision(kCGEventSourceStateCombinedSessionState);
        out->hardware_events = event_revision(kCGEventSourceStateHIDSystemState);
        out->physical_input_down = (CGEventSourceFlagsState(kCGEventSourceStateHIDSystemState)
            & (kCGEventFlagMaskShift | kCGEventFlagMaskControl | kCGEventFlagMaskAlternate | kCGEventFlagMaskCommand)) != 0;
        for (CGMouseButton button = 0; button < 3; button++) out->physical_input_down |= CGEventSourceButtonState(kCGEventSourceStateHIDSystemState, button);
        for (CGKeyCode key = 0; key < 128; key++) if (key != 57) out->physical_input_down |= CGEventSourceKeyState(kCGEventSourceStateHIDSystemState, key);
        CGDirectDisplayID displays[32];
        uint32_t displayCount = 0;
        if (CGGetActiveDisplayList(32, displays, &displayCount) == kCGErrorSuccess && displayCount < 32) {
            uint64_t layout = 1469598103934665603ULL;
            for (uint32_t i = 0; i < displayCount; i++) {
                CGRect rect = CGDisplayBounds(displays[i]);
                uint64_t fields[] = { displays[i], (uint64_t)(int64_t)rect.origin.x,
                    (uint64_t)(int64_t)rect.origin.y, (uint64_t)rect.size.width,
                    (uint64_t)rect.size.height, CGDisplayPixelsWide(displays[i]), CGDisplayPixelsHigh(displays[i]) };
                for (size_t j = 0; j < sizeof(fields) / sizeof(fields[0]); j++) layout = (layout ^ fields[j]) * 1099511628211ULL;
            }
            out->layout = layout;
        }
        out->focus_pid = front.processIdentifier;
        if (out->input && out->focus_pid > 0) {
            AXUIElementRef app = AXUIElementCreateApplication(out->focus_pid);
            AXUIElementSetMessagingTimeout(app, 0.2f);
            CFTypeRef focused = NULL;
            if (AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute, &focused) == kAXErrorSuccess && focused) {
                out->focus = CFHash(focused);
                AXUIElementSetMessagingTimeout((AXUIElementRef)focused, 0.2f);
                CFTypeRef position = NULL;
                CFTypeRef size = NULL;
                if (AXUIElementCopyAttributeValue((AXUIElementRef)focused, kAXPositionAttribute, &position) == kAXErrorSuccess
                    && AXUIElementCopyAttributeValue((AXUIElementRef)focused, kAXSizeAttribute, &size) == kAXErrorSuccess
                    && CFGetTypeID(position) == AXValueGetTypeID() && CFGetTypeID(size) == AXValueGetTypeID()) {
                    CGPoint origin;
                    CGSize dimensions;
                    if (AXValueGetValue(position, kAXValueCGPointType, &origin)
                        && AXValueGetValue(size, kAXValueCGSizeType, &dimensions)) {
                        out->focus_on_display = CGRectContainsRect(bounds, (CGRect){origin, dimensions});
                    }
                }
                if (position) CFRelease(position);
                if (size) CFRelease(size);
                CFRelease(focused);
            }
            CFRelease(app);
        }
        if (out->width == 0 || out->height == 0 || bounds.size.width <= 0 || bounds.size.height <= 0 || !out->layout) {
            out->ready = 0;
        }
    }
}

@interface BuckyCapture : NSObject
@property dispatch_semaphore_t semaphore;
@property NSData *data;
@end
@implementation BuckyCapture
@end

int bucky_desktop_capture(uint32_t displayID, uint32_t width, uint32_t height,
                          uint64_t timeoutMs, unsigned char **bytes, size_t *length) {
    @autoreleasepool {
        *bytes = NULL;
        *length = 0;
        if (!CGPreflightScreenCaptureAccess()) return 1;
        if (atomic_exchange(&capture_pending, true)) return 4;
        BuckyCapture *capture = [BuckyCapture new];
        capture.semaphore = dispatch_semaphore_create(0);
        [SCShareableContent getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:YES completionHandler:^(SCShareableContent *content, NSError *error) {
            SCDisplay *selected = nil;
            for (SCDisplay *display in content.displays) {
                if (display.displayID == displayID) { selected = display; break; }
            }
            if (error || !selected) {
                atomic_store(&capture_pending, false);
                dispatch_semaphore_signal(capture.semaphore);
                return;
            }
            SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:selected excludingWindows:@[]];
            SCStreamConfiguration *configuration = [SCStreamConfiguration new];
            configuration.width = width;
            configuration.height = height;
            configuration.showsCursor = NO;
            [SCScreenshotManager captureImageWithFilter:filter configuration:configuration completionHandler:^(CGImageRef image, NSError *imageError) {
                if (image && !imageError && CGImageGetWidth(image) == width && CGImageGetHeight(image) == height) {
                    NSMutableData *png = [NSMutableData data];
                    CGImageDestinationRef destination = CGImageDestinationCreateWithData((__bridge CFMutableDataRef)png, CFSTR("public.png"), 1, NULL);
                    if (destination) {
                        CGImageDestinationAddImage(destination, image, NULL);
                        if (CGImageDestinationFinalize(destination) && png.length <= 16 * 1024 * 1024) {
                            capture.data = png;
                        }
                        CFRelease(destination);
                    }
                }
                atomic_store(&capture_pending, false);
                dispatch_semaphore_signal(capture.semaphore);
            }];
        }];
        if (dispatch_semaphore_wait(capture.semaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)timeoutMs * NSEC_PER_MSEC))) return 2;
        if (!capture.data) return 3;
        *length = capture.data.length;
        *bytes = malloc(*length);
        if (!*bytes) return 3;
        memcpy(*bytes, capture.data.bytes, *length);
        return 0;
    }
}

int bucky_desktop_click(double x, double y, uint32_t button, uint32_t count, uint64_t flags) {
    CGPoint point = CGPointMake(x, y);
    CGEventType downType = button == 0 ? kCGEventLeftMouseDown : button == 1 ? kCGEventRightMouseDown : kCGEventOtherMouseDown;
    CGEventType upType = button == 0 ? kCGEventLeftMouseUp : button == 1 ? kCGEventRightMouseUp : kCGEventOtherMouseUp;
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStatePrivate);
    if (!source) return 1;
    CGEventRef down = CGEventCreateMouseEvent(source, downType, point, button);
    CGEventRef up = CGEventCreateMouseEvent(source, upType, point, button);
    CFRelease(source);
    if (!down || !up) { if (down) CFRelease(down); if (up) CFRelease(up); return 1; }
    CGEventSetFlags(down, flags);
    CGEventSetFlags(up, CGEventSourceFlagsState(kCGEventSourceStateHIDSystemState));
    CGEventSetIntegerValueField(down, kCGMouseEventClickState, count);
    CGEventSetIntegerValueField(up, kCGMouseEventClickState, count);
    CGEventPost(kCGSessionEventTap, down);
    CGEventPost(kCGSessionEventTap, up);
    CFRelease(down);
    CFRelease(up);
    return 0;
}

int bucky_desktop_key(uint16_t key, uint64_t flags, const uint16_t *text, size_t length) {
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStatePrivate);
    if (!source) return 1;
    CGEventRef down = CGEventCreateKeyboardEvent(source, key, true);
    CGEventRef up = CGEventCreateKeyboardEvent(source, key, false);
    CFRelease(source);
    if (!down || !up) { if (down) CFRelease(down); if (up) CFRelease(up); return 1; }
    CGEventSetFlags(down, flags);
    CGEventSetFlags(up, CGEventSourceFlagsState(kCGEventSourceStateHIDSystemState));
    if (length) {
        CGEventKeyboardSetUnicodeString(down, length, text);
        CGEventKeyboardSetUnicodeString(up, length, text);
    }
    CGEventPost(kCGSessionEventTap, down);
    CGEventPost(kCGSessionEventTap, up);
    CFRelease(down);
    CFRelease(up);
    return 0;
}

int bucky_desktop_keycode(uint16_t character) {
    TISInputSourceRef source = TISCopyCurrentKeyboardLayoutInputSource();
    if (!source) return -1;
    CFDataRef data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
    int result = -1;
    if (data) {
        const UCKeyboardLayout *layout = (const UCKeyboardLayout *)CFDataGetBytePtr(data);
        for (UInt16 key = 0; key < 128; key++) {
            UInt32 state = 0;
            UniChar chars[8];
            UniCharCount length = 0;
            OSStatus error = UCKeyTranslate(layout, key, kUCKeyActionDown, 0, LMGetKbdType(),
                kUCKeyTranslateNoDeadKeysBit, &state, 8, &length, chars);
            if (error == noErr && length == 1 && chars[0] == character) { result = key; break; }
        }
    }
    CFRelease(source);
    return result;
}

int bucky_desktop_scroll(double x, double y, int32_t deltaX, int32_t deltaY, uint32_t pixel) {
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStatePrivate);
    if (!source) return 1;
    CGEventRef event = CGEventCreateScrollWheelEvent(source, pixel ? kCGScrollEventUnitPixel : kCGScrollEventUnitLine, 2, -deltaY, -deltaX);
    CFRelease(source);
    if (!event) return 1;
    CGEventSetLocation(event, CGPointMake(x, y));
    CGEventPost(kCGSessionEventTap, event);
    CFRelease(event);
    return 0;
}

int bucky_desktop_release(uint16_t cleanup) {
    if (cleanup == UINT16_MAX) return 0;
    CGEventSourceRef source = CGEventSourceCreate(kCGEventSourceStatePrivate);
    if (!source) return 1;
    CGEventRef up = NULL;
    if (cleanup < 128) {
        up = CGEventCreateKeyboardEvent(source, cleanup, false);
    } else if (cleanup >= 256 && cleanup <= 258) {
        CGEventRef current = CGEventCreate(NULL);
        if (current) {
            CGMouseButton button = cleanup - 256;
            CGEventType type = button == 0 ? kCGEventLeftMouseUp : button == 1 ? kCGEventRightMouseUp : kCGEventOtherMouseUp;
            up = CGEventCreateMouseEvent(source, type, CGEventGetLocation(current), button);
            CFRelease(current);
        }
    }
    CFRelease(source);
    if (!up) return 1;
    CGEventSetFlags(up, CGEventSourceFlagsState(kCGEventSourceStateHIDSystemState));
    CGEventPost(kCGSessionEventTap, up);
    CFRelease(up);
    return 0;
}
