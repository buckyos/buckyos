#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Carbon/Carbon.h>
#include <unistd.h>
#include <stdatomic.h>
#include <math.h>
#include <libproc.h>
#include <os/lock.h>
#include <bsm/audit.h>

static atomic_bool capture_pending = false;

void bucky_desktop_initialize(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    }
}

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
        out->capture = CGPreflightScreenCaptureAccess();
        out->input = AXIsProcessTrusted() && CGPreflightPostEventAccess();
        AXUIElementRef focusedApp = NULL;
        if (out->input) {
            AXUIElementRef system = AXUIElementCreateSystemWide();
            AXUIElementSetMessagingTimeout(system, 0.2f);
            CFTypeRef value = NULL;
            if (AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute, &value) == kAXErrorSuccess
                && value && CFGetTypeID(value) == AXUIElementGetTypeID()) {
                pid_t pid = 0;
                if (AXUIElementGetPid((AXUIElementRef)value, &pid) == kAXErrorSuccess && pid > 0) {
                    focusedApp = (AXUIElementRef)value;
                    out->focus_pid = pid;
                }
            }
            if (value && !focusedApp) CFRelease(value);
            CFRelease(system);
        }
        NSRunningApplication *front = out->focus_pid > 0
            ? [NSRunningApplication runningApplicationWithProcessIdentifier:out->focus_pid] : nil;
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
        if (focusedApp) {
            AXUIElementRef app = focusedApp;
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
                        // A scrolled text area reports its whole document:
                        // what shows of it is the part inside its window.
                        CGRect shown = (CGRect){origin, dimensions};
                        CFTypeRef window = NULL;
                        if (AXUIElementCopyAttributeValue((AXUIElementRef)focused, kAXWindowAttribute, &window) == kAXErrorSuccess
                            && window && CFGetTypeID(window) == AXUIElementGetTypeID()) {
                            AXUIElementSetMessagingTimeout((AXUIElementRef)window, 0.2f);
                            CFTypeRef windowPosition = NULL;
                            CFTypeRef windowSize = NULL;
                            CGPoint windowOrigin;
                            CGSize windowDimensions;
                            if (AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXPositionAttribute, &windowPosition) == kAXErrorSuccess
                                && AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXSizeAttribute, &windowSize) == kAXErrorSuccess
                                && AXValueGetValue(windowPosition, kAXValueCGPointType, &windowOrigin)
                                && AXValueGetValue(windowSize, kAXValueCGSizeType, &windowDimensions)) {
                                shown = CGRectIntersection(shown, (CGRect){windowOrigin, windowDimensions});
                            }
                            if (windowPosition) CFRelease(windowPosition);
                            if (windowSize) CFRelease(windowSize);
                        }
                        if (window) CFRelease(window);
                        out->focus_on_display = !CGRectIsEmpty(shown) && CGRectContainsRect(bounds, shown);
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

void bucky_desktop_request_access(uint32_t *capture, uint32_t *input) {
    @autoreleasepool {
        *capture = CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess();
        NSDictionary *options = @{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES};
        BOOL trusted = AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
        BOOL post = CGPreflightPostEventAccess() || CGRequestPostEventAccess();
        *input = trusted && post;
    }
}

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

// Semantic observation through macOS Accessibility: the bucky_semantic_* ABI
// shared with the UI Automation backend in native_windows.cpp (bits, result
// codes and JSON fields match semantic.rs and desktop.rs). AX calls are
// synchronous IPC to the target app and may run on any thread; every element
// gets a messaging timeout and a snapshot also has an overall deadline.

extern AXError _AXUIElementGetWindow(AXUIElementRef element, CGWindowID *window);

enum {
    PAT_INVOKE = 1u << 0, PAT_VALUE = 1u << 1, PAT_TOGGLE = 1u << 2, PAT_SELECTION_ITEM = 1u << 3,
    PAT_EXPAND_COLLAPSE = 1u << 4, PAT_SCROLL_ITEM = 1u << 5, PAT_FOCUSABLE = 1u << 6,
};
enum {
    ST_DISABLED = 1u << 0, ST_FOCUSED = 1u << 1, ST_READONLY = 1u << 2, ST_SELECTED = 1u << 3,
    ST_CHECKED = 1u << 4, ST_MIXED = 1u << 5, ST_EXPANDED = 1u << 6, ST_COLLAPSED = 1u << 7,
};
enum { UR_OK = 0, UR_STALE = 1, UR_TIMEOUT = 2, UR_FAILED = 3, UR_UNAVAILABLE = 4, UR_DISABLED = 6 };

static const uint32_t AX_MAX_VISITED = 3000;
static const uint32_t AX_MAX_DEPTH = 256;
static const NSUInteger AX_MAX_CHILDREN = 1000;
static const NSUInteger AX_MAX_NAME = 512;
static const NSUInteger AX_MAX_VALUE = 1024;
static const NSUInteger AX_MAX_READBACK = 20000;
static const NSUInteger AX_MAX_SNAPSHOTS = 96;
static const NSUInteger AX_MAX_WINDOWS = 32;
static const NSUInteger AX_MAX_PARENTS = 64;
static const float AX_CALL_TIMEOUT = 1.0f;

@interface BuckyAxNode : NSObject
@property (nonatomic, strong) id element;
@property (nonatomic, strong) id window;
@property (nonatomic, copy) NSString *role;
@property (nonatomic) pid_t pid;
@end
@implementation BuckyAxNode
@end

@interface BuckyAxFacts : NSObject
@property (nonatomic, copy) NSString *role;
@property (nonatomic, copy) NSString *subrole;
@property (nonatomic, copy) NSString *name;
@property (nonatomic, copy) NSString *identifier;
@property (nonatomic, copy) NSString *value;
@property (nonatomic, copy) NSString *expander;
@property (nonatomic) uint32_t patterns;
@property (nonatomic) uint32_t states;
@property (nonatomic) CGRect frame;
@property (nonatomic, strong) NSArray *children;
@end
@implementation BuckyAxFacts
@end

@interface BuckyAxVisit : NSObject
@property (nonatomic, strong) NSArray *children;
@property (nonatomic, strong) id window;
@property (nonatomic) int parent;
@property (nonatomic) int label;
@property (nonatomic) uint32_t depth;
@property (nonatomic) CGRect clip;
@end
@implementation BuckyAxVisit
@end

static os_unfair_lock ax_lock = OS_UNFAIR_LOCK_INIT;
static NSMutableDictionary<NSNumber *, NSMutableArray *> *ax_snapshots;
static uint64_t ax_next_snapshot = 1;

#define AX_SET(name, ...) static NSSet *name(void) { \
    static NSSet *set; static dispatch_once_t once; \
    dispatch_once(&once, ^{ set = [NSSet setWithArray:@[__VA_ARGS__]]; }); return set; }

AX_SET(ax_textual_roles, @"AXTextField", @"AXTextArea", @"AXComboBox", @"AXDateField", @"AXTimeField")
AX_SET(ax_valued_roles, @"AXTextField", @"AXTextArea", @"AXComboBox", @"AXDateField", @"AXTimeField",
    @"AXPopUpButton", @"AXSlider")
AX_SET(ax_focus_roles, @"AXTextField", @"AXTextArea", @"AXComboBox", @"AXDateField", @"AXTimeField",
    @"AXList", @"AXTable", @"AXOutline", @"AXBrowser", @"AXGrid", @"AXWebArea", @"AXSlider")
AX_SET(ax_actionless_roles, @"AXStaticText", @"AXScrollArea", @"AXScrollBar", @"AXSplitter", @"AXSplitGroup",
    @"AXColumn", @"AXValueIndicator", @"AXProgressIndicator", @"AXBusyIndicator", @"AXMenuBar", @"AXMenu",
    @"AXWindow", @"AXSheet", @"AXApplication", @"AXTabGroup", @"AXLayoutArea", @"AXUnknown")
AX_SET(ax_leaf_roles, @"AXStaticText", @"AXImage", @"AXScrollBar", @"AXValueIndicator", @"AXBusyIndicator",
    @"AXProgressIndicator", @"AXSplitter", @"AXGrowArea", @"AXColumn", @"AXLevelIndicator",
    @"AXRelevanceIndicator", @"AXHandle", @"AXTextField", @"AXTextArea", @"AXDateField", @"AXTimeField",
    @"AXSlider")
AX_SET(ax_clip_roles, @"AXScrollArea", @"AXWebArea")
AX_SET(ax_expandable_roles, @"AXComboBox", @"AXPopUpButton", @"AXMenuButton", @"AXRow")
AX_SET(ax_meaningful_roles, @"AXWindow", @"AXSheet", @"AXPopover", @"AXWebArea", @"AXToolbar", @"AXHeading",
    @"AXCheckBox", @"AXButton", @"AXMenuButton", @"AXDisclosureTriangle", @"AXColorWell", @"AXDockItem",
    @"AXLink", @"AXRadioButton", @"AXTabGroup", @"AXMenu", @"AXMenuBar", @"AXMenuItem", @"AXMenuBarItem",
    @"AXTextField", @"AXTextArea", @"AXDateField", @"AXTimeField", @"AXComboBox", @"AXPopUpButton",
    @"AXSlider", @"AXList", @"AXOutline", @"AXBrowser", @"AXRow", @"AXTable", @"AXGrid", @"AXCell")

static BOOL ax_meaningful(NSString *role, NSString *subrole) {
    if ([role isEqualToString:@"AXGroup"]) {
        return [subrole isEqualToString:@"AXApplicationDialog"] || [subrole isEqualToString:@"AXApplicationAlertDialog"]
            || [subrole isEqualToString:@"AXDocument"];
    }
    return [ax_meaningful_roles() containsObject:role];
}

static int ax_status(AXError error) {
    switch (error) {
        case kAXErrorSuccess: return UR_OK;
        case kAXErrorInvalidUIElement: return UR_STALE;
        case kAXErrorCannotComplete: return UR_TIMEOUT;
        case kAXErrorAPIDisabled: return UR_UNAVAILABLE;
        default: return UR_FAILED;
    }
}

static double ax_now(void) {
    return (double)clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e9;
}

static id ax_present(id value) {
    if (!value || value == (id)[NSNull null]) return nil;
    CFTypeRef ref = (__bridge CFTypeRef)value;
    if (CFGetTypeID(ref) == AXValueGetTypeID() && AXValueGetType((AXValueRef)ref) == kAXValueTypeAXError) return nil;
    return value;
}

/// Bounded text with lone surrogates replaced, so it always encodes as JSON.
static NSString *ax_text(id value, NSUInteger limit) {
    value = ax_present(value);
    NSString *text = nil;
    if ([value isKindOfClass:[NSString class]]) text = value;
    else if ([value isKindOfClass:[NSAttributedString class]]) text = [value string];
    else if ([value isKindOfClass:[NSNumber class]] && CFGetTypeID((__bridge CFTypeRef)value) != CFBooleanGetTypeID())
        text = [value stringValue];
    if (!text.length) return @"";
    NSUInteger length = MIN(text.length, limit);
    if (length < text.length && CFStringIsSurrogateHighCharacter([text characterAtIndex:length - 1])) length--;
    unichar *buffer = malloc(length * sizeof(unichar) + 1);
    if (!buffer) return @"";
    [text getCharacters:buffer range:NSMakeRange(0, length)];
    for (NSUInteger i = 0; i < length; i++) {
        BOOL high = CFStringIsSurrogateHighCharacter(buffer[i]);
        if (high && i + 1 < length && CFStringIsSurrogateLowCharacter(buffer[i + 1])) { i++; continue; }
        if (high || CFStringIsSurrogateLowCharacter(buffer[i])) buffer[i] = 0xFFFD;
    }
    NSString *result = [[NSString alloc] initWithCharacters:buffer length:length];
    free(buffer);
    return result;
}

static int ax_bool(id value) {
    value = ax_present(value);
    return [value isKindOfClass:[NSNumber class]] ? ([value boolValue] ? 1 : 0) : -1;
}

static CGRect ax_frame(id position, id size) {
    position = ax_present(position);
    size = ax_present(size);
    CGPoint origin;
    CGSize extent;
    if (!position || !size || CFGetTypeID((__bridge CFTypeRef)position) != AXValueGetTypeID()
        || CFGetTypeID((__bridge CFTypeRef)size) != AXValueGetTypeID()
        || !AXValueGetValue((__bridge AXValueRef)position, kAXValueTypeCGPoint, &origin)
        || !AXValueGetValue((__bridge AXValueRef)size, kAXValueTypeCGSize, &extent)
        || !isfinite(origin.x) || !isfinite(origin.y) || !isfinite(extent.width) || !isfinite(extent.height)) {
        return CGRectNull;
    }
    return CGRectMake(origin.x, origin.y, extent.width, extent.height);
}

static BOOL ax_geometry(CGRect frame) {
    return !CGRectIsNull(frame) && frame.size.width > 0 && frame.size.height > 0;
}

static NSArray *ax_rect(CGRect rect) {
    if (CGRectIsNull(rect)) return @[@0, @0, @0, @0];
    double left = floor(rect.origin.x), top = floor(rect.origin.y);
    double right = ceil(rect.origin.x + rect.size.width), bottom = ceil(rect.origin.y + rect.size.height);
    return @[@((int64_t)left), @((int64_t)top), @((int64_t)fmax(right - left, 0)), @((int64_t)fmax(bottom - top, 0))];
}

static CGRect ax_display(void) {
    return CGDisplayBounds(CGMainDisplayID());
}

static BOOL ax_alive(AXError error) {
    return error == kAXErrorSuccess || error == kAXErrorNoValue || error == kAXErrorAttributeUnsupported;
}

static BOOL ax_settable(AXUIElementRef element, CFStringRef attribute) {
    Boolean settable = false;
    return AXUIElementIsAttributeSettable(element, attribute, &settable) == kAXErrorSuccess && settable;
}

enum {
    A_ROLE, A_SUBROLE, A_TITLE, A_DESCRIPTION, A_VALUE, A_IDENTIFIER, A_ENABLED, A_ROLE_DESCRIPTION, A_SELECTED,
    A_EXPANDED, A_POSITION, A_SIZE, A_HELP, A_PLACEHOLDER, A_DOM_ID, A_MARK, A_TITLE_ELEMENT, A_DISCLOSING, A_CHILDREN,
};

static NSArray *ax_attributes(BOOL children) {
    static NSArray *facts, *withChildren;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        facts = @[(__bridge NSString *)kAXRoleAttribute, (__bridge NSString *)kAXSubroleAttribute,
            (__bridge NSString *)kAXTitleAttribute, (__bridge NSString *)kAXDescriptionAttribute,
            (__bridge NSString *)kAXValueAttribute, (__bridge NSString *)kAXIdentifierAttribute,
            (__bridge NSString *)kAXEnabledAttribute, (__bridge NSString *)kAXRoleDescriptionAttribute,
            (__bridge NSString *)kAXSelectedAttribute, (__bridge NSString *)kAXExpandedAttribute,
            (__bridge NSString *)kAXPositionAttribute, (__bridge NSString *)kAXSizeAttribute,
            (__bridge NSString *)kAXHelpAttribute, (__bridge NSString *)kAXPlaceholderValueAttribute,
            @"AXDOMIdentifier", (__bridge NSString *)kAXMenuItemMarkCharAttribute,
            (__bridge NSString *)kAXTitleUIElementAttribute, (__bridge NSString *)kAXDisclosingAttribute];
        withChildren = [facts arrayByAddingObject:(__bridge NSString *)kAXChildrenAttribute];
    });
    return children ? withChildren : facts;
}

/// One element's role, name, geometry and pattern/state bits (the same bits
/// the UIA backend derives from control patterns), plus its children.
/// Toggle, radio select and disclosure triangles need AXPress; row select
/// and expand/collapse need a settable AXSelected, AXExpanded or (outline
/// rows) AXDisclosing, recorded in `expander`.
/// Focus is not read here: `AXFocused` is true on every cell of a focused
/// SwiftUI list, so callers compare with the app's focused element.
static BuckyAxFacts *ax_read(AXUIElementRef element, BOOL children, float timeout, AXError *error) {
    AXUIElementSetMessagingTimeout(element, timeout);
    CFArrayRef raw = NULL;
    *error = AXUIElementCopyMultipleAttributeValues(element, (__bridge CFArrayRef)ax_attributes(children), 0, &raw);
    NSArray *values = CFBridgingRelease(raw);
    if (*error != kAXErrorSuccess || values.count < (children ? A_CHILDREN + 1 : A_CHILDREN)) {
        if (*error == kAXErrorSuccess) *error = kAXErrorFailure;
        return nil;
    }
    BuckyAxFacts *facts = [BuckyAxFacts new];
    NSString *role = ax_text(values[A_ROLE], 64);
    if (!role.length) {
        *error = kAXErrorInvalidUIElement;
        return nil;
    }
    facts.role = role;
    facts.subrole = ax_text(values[A_SUBROLE], 64);
    facts.frame = ax_frame(values[A_POSITION], values[A_SIZE]);
    if (children) {
        id list = ax_present(values[A_CHILDREN]);
        facts.children = [list isKindOfClass:[NSArray class]] ? list : @[];
    }
    uint32_t patterns = 0, states = 0;
    BOOL inert = [ax_actionless_roles() containsObject:role];
    if (!inert) {
        CFArrayRef names = NULL;
        if (AXUIElementCopyActionNames(element, &names) == kAXErrorSuccess) {
            NSArray *actions = CFBridgingRelease(names);
            if ([actions containsObject:(__bridge NSString *)kAXPressAction]) patterns |= PAT_INVOKE;
            if ([actions containsObject:@"AXScrollToVisible"]) patterns |= PAT_SCROLL_ITEM;
        }
    }
    if (ax_bool(values[A_ENABLED]) == 0) states |= ST_DISABLED;
    BOOL menu = [role isEqualToString:@"AXMenuItem"] || [role isEqualToString:@"AXMenuBarItem"];
    BOOL row = [role isEqualToString:@"AXRow"];
    BOOL press = (patterns & PAT_INVOKE) != 0;
    // Chromium reports AXSelected/AXExpanded (false, read-only) on every
    // node, and SwiftUI and Finder report read-only AXDisclosing on rows
    // that do not expand: the patterns need a settable attribute on roles
    // that select or expand, other roles only show a true state.
    int selected = ax_bool(values[A_SELECTED]);
    if (selected >= 0 && !menu && !inert) {
        if (row && ax_settable(element, kAXSelectedAttribute)) patterns |= PAT_SELECTION_ITEM;
        if (selected == 1) states |= ST_SELECTED;
    }
    id value = ax_present(values[A_VALUE]);
    int number = [value isKindOfClass:[NSNumber class]] ? [value intValue] : -1;
    if ([role isEqualToString:@"AXCheckBox"]) {
        if (press) patterns |= PAT_TOGGLE;
        if (number == 1) states |= ST_CHECKED;
        else if (number == 2) states |= ST_MIXED;
    } else if ([role isEqualToString:@"AXRadioButton"]) {
        if (press) patterns |= PAT_SELECTION_ITEM;
        if (number == 1) states |= ST_SELECTED;
    } else if ([role isEqualToString:@"AXMenuItem"]) {
        NSString *mark = ax_text(values[A_MARK], 4);
        if (mark.length) states |= [mark isEqualToString:@"-"] ? ST_MIXED : ST_CHECKED;
    } else if ([role isEqualToString:@"AXDisclosureTriangle"] && number >= 0) {
        if (press) patterns |= PAT_EXPAND_COLLAPSE;
        states |= number ? ST_EXPANDED : ST_COLLAPSED;
    }
    int expanded = ax_bool(values[A_EXPANDED]);
    if (!menu && !inert) {
        CFStringRef expander = NULL;
        int open = expanded;
        if ([ax_expandable_roles() containsObject:role]) {
            if (expanded >= 0) expander = kAXExpandedAttribute;
            else if (row && (open = ax_bool(values[A_DISCLOSING])) >= 0) expander = kAXDisclosingAttribute;
        }
        if (expander && ax_settable(element, expander)) {
            patterns |= PAT_EXPAND_COLLAPSE;
            states |= open ? ST_EXPANDED : ST_COLLAPSED;
            facts.expander = (__bridge NSString *)expander;
        } else if (expanded == 1) {
            states |= ST_EXPANDED;
        }
    }
    BOOL textual = [ax_textual_roles() containsObject:role];
    if (textual) {
        if (ax_settable(element, kAXValueAttribute)) patterns |= PAT_VALUE;
        else states |= ST_READONLY;
    }
    if ([ax_focus_roles() containsObject:role] && ax_settable(element, kAXFocusedAttribute)) patterns |= PAT_FOCUSABLE;
    facts.patterns = patterns;
    facts.states = states;
    if ([ax_valued_roles() containsObject:role]) facts.value = ax_text(value, AX_MAX_VALUE);
    NSString *name = ax_text(values[A_TITLE], AX_MAX_NAME);
    if (!name.length) name = ax_text(values[A_DESCRIPTION], AX_MAX_NAME);
    if (!name.length && [role isEqualToString:@"AXStaticText"]) name = ax_text(value, AX_MAX_NAME);
    id label = ax_present(values[A_TITLE_ELEMENT]);
    if (!name.length && label && CFGetTypeID((__bridge CFTypeRef)label) == AXUIElementGetTypeID()) {
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)label, timeout);
        CFTypeRef text = NULL;
        if (AXUIElementCopyAttributeValue((__bridge AXUIElementRef)label, kAXValueAttribute, &text) == kAXErrorSuccess) {
            name = ax_text(CFBridgingRelease(text), AX_MAX_NAME);
        }
    }
    if (!name.length && textual) name = ax_text(values[A_PLACEHOLDER], AX_MAX_NAME);
    if (!name.length && facts.subrole.length && ((patterns & PAT_INVOKE) || textual)) name = ax_text(values[A_ROLE_DESCRIPTION], AX_MAX_NAME);
    if (!name.length && (patterns & ~PAT_SCROLL_ITEM)) name = ax_text(values[A_HELP], AX_MAX_NAME);
    if (!name.length && [role isEqualToString:@"AXMenuItem"] && (states & ST_DISABLED)) facts.patterns = 0;
    facts.name = name;
    NSString *identifier = ax_text(values[A_IDENTIFIER], 128);
    if (!identifier.length || [identifier hasPrefix:@"_NS:"]) identifier = ax_text(values[A_DOM_ID], 128);
    facts.identifier = identifier;
    return facts;
}

static CGWindowID ax_window_number(AXUIElementRef window, CGRect frame, pid_t pid, NSArray *listed) {
    CGWindowID number = 0;
    if (_AXUIElementGetWindow(window, &number) == kAXErrorSuccess && number) return number;
    for (NSDictionary *entry in listed) {
        CGRect bounds = [entry[@"frame"] rectValue];
        if ([entry[@"pid"] intValue] == pid && fabs(bounds.origin.x - frame.origin.x) <= 1 && fabs(bounds.origin.y - frame.origin.y) <= 1
            && fabs(bounds.size.width - frame.size.width) <= 1 && fabs(bounds.size.height - frame.size.height) <= 1) {
            return [entry[@"id"] unsignedIntValue];
        }
    }
    return 0;
}

static NSArray *ax_app_windows(pid_t pid, float timeout, AXError *error) {
    AXUIElementRef app = AXUIElementCreateApplication(pid);
    if (!app) { *error = kAXErrorFailure; return @[]; }
    AXUIElementSetMessagingTimeout(app, timeout);
    CFTypeRef windows = NULL;
    *error = AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &windows);
    CFRelease(app);
    id list = CFBridgingRelease(windows);
    return *error == kAXErrorSuccess && [list isKindOfClass:[NSArray class]] ? list : @[];
}

/// The on-screen windows of the main display, front to back (normal window
/// layer, visible, not minimized or on another Space).
static NSMutableArray *ax_screen_windows(CGRect display) {
    NSArray *list = CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    NSMutableArray *windows = [NSMutableArray array];
    for (NSDictionary *info in list) {
        CGRect frame;
        if ([info[(__bridge NSString *)kCGWindowLayer] intValue] != 0
            || [info[(__bridge NSString *)kCGWindowAlpha] doubleValue] <= 0
            || !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)info[(__bridge NSString *)kCGWindowBounds], &frame)
            || frame.size.width < 2 || frame.size.height < 2 || !CGRectIntersectsRect(frame, display)) {
            continue;
        }
        [windows addObject:@{
            @"id": info[(__bridge NSString *)kCGWindowNumber] ?: @0,
            @"pid": info[(__bridge NSString *)kCGWindowOwnerPID] ?: @0,
            @"title": info[(__bridge NSString *)kCGWindowName] ?: @"",
            @"process": info[(__bridge NSString *)kCGWindowOwnerName] ?: @"",
            @"frame": [NSValue valueWithRect:frame],
        }];
    }
    return windows;
}

static BOOL ax_same_user(pid_t pid) {
    struct proc_bsdshortinfo info;
    return proc_pidinfo(pid, PROC_PIDT_SHORTBSDINFO, 0, &info, sizeof(info)) == sizeof(info) && info.pbsi_uid == geteuid();
}

static pid_t ax_focused_pid(void) {
    AXUIElementRef system = AXUIElementCreateSystemWide();
    AXUIElementSetMessagingTimeout(system, 0.25f);
    CFTypeRef app = NULL;
    pid_t pid = 0;
    if (AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute, &app) == kAXErrorSuccess && app
        && CFGetTypeID(app) == AXUIElementGetTypeID()) {
        AXUIElementGetPid((AXUIElementRef)app, &pid);
    }
    if (app) CFRelease(app);
    CFRelease(system);
    return pid;
}

static id ax_copy(AXUIElementRef element, CFStringRef attribute) {
    CFTypeRef value = NULL;
    return AXUIElementCopyAttributeValue(element, attribute, &value) == kAXErrorSuccess ? CFBridgingRelease(value) : nil;
}

static BOOL ax_is_element(id value) {
    return value && CFGetTypeID((__bridge CFTypeRef)value) == AXUIElementGetTypeID();
}

static NSData *ax_json(id object) {
    return [NSJSONSerialization dataWithJSONObject:object options:NSJSONWritingWithoutEscapingSlashes error:nil];
}

static int ax_reply(NSData *data, char **json, size_t *length) {
    if (!data) return UR_FAILED;
    *json = malloc(data.length + 1);
    if (!*json) return UR_FAILED;
    memcpy(*json, data.bytes, data.length);
    (*json)[data.length] = 0;
    *length = data.length;
    return UR_OK;
}

int bucky_semantic_status(void) {
    return AXIsProcessTrusted() ? UR_OK : UR_UNAVAILABLE;
}

int bucky_semantic_snapshot(uint64_t root, uint32_t root_pid, uint32_t max_nodes, uint32_t budget_ms, char **json, size_t *length) {
    if (!json || !length) return UR_FAILED;
    *json = NULL;
    *length = 0;
    @autoreleasepool {
        if (!AXIsProcessTrusted()) return UR_UNAVAILABLE;
        double deadline = ax_now() + budget_ms / 1000.0;
        CGRect display = ax_display();
        NSMutableArray *screen = ax_screen_windows(display);
        pid_t focused = ax_focused_pid();
        pid_t pid = root ? (pid_t)root_pid : focused;
        __block NSString *reason = nil;
        id window = nil;
        CGWindowID number = 0;
        AXError error = kAXErrorSuccess;
        if (root) {
            CGRect frame = CGRectNull;
            for (NSDictionary *entry in screen) {
                if ([entry[@"id"] unsignedLongLongValue] == root && [entry[@"pid"] intValue] == pid) frame = [entry[@"frame"] rectValue];
            }
            if (CGRectIsNull(frame)) return UR_STALE;
            number = (CGWindowID)root;
            if (!ax_same_user(pid)) reason = @"window_inaccessible";
            else {
                for (id candidate in ax_app_windows(pid, AX_CALL_TIMEOUT, &error)) {
                    AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)candidate, AX_CALL_TIMEOUT);
                    CGRect bounds = ax_frame(ax_copy((__bridge AXUIElementRef)candidate, kAXPositionAttribute),
                        ax_copy((__bridge AXUIElementRef)candidate, kAXSizeAttribute));
                    if (ax_window_number((__bridge AXUIElementRef)candidate, bounds, pid, @[@{@"id": @(number), @"pid": @(pid),
                            @"frame": [NSValue valueWithRect:frame]}]) == number) {
                        window = candidate;
                        break;
                    }
                }
                if (!window) reason = error == kAXErrorCannotComplete ? @"backend_timeout" : @"window_inaccessible";
            }
        } else if (!pid) {
            reason = @"no_window";
        } else if (!ax_same_user(pid)) {
            reason = @"window_inaccessible";
        } else {
            AXUIElementRef app = AXUIElementCreateApplication(pid);
            AXUIElementSetMessagingTimeout(app, AX_CALL_TIMEOUT);
            CFTypeRef value = NULL;
            error = AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute, &value);
            if (error != kAXErrorSuccess || !value) error = AXUIElementCopyAttributeValue(app, kAXMainWindowAttribute, &value);
            if (value && CFGetTypeID(value) == AXUIElementGetTypeID()) window = CFBridgingRelease(value);
            else if (value) CFRelease(value);
            if (!window && error != kAXErrorCannotComplete) window = ax_app_windows(pid, AX_CALL_TIMEOUT, &error).firstObject;
            CFRelease(app);
            if (window) {
                AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)window, AX_CALL_TIMEOUT);
                CGRect bounds = ax_frame(ax_copy((__bridge AXUIElementRef)window, kAXPositionAttribute),
                    ax_copy((__bridge AXUIElementRef)window, kAXSizeAttribute));
                number = ax_window_number((__bridge AXUIElementRef)window, bounds, pid, screen);
            } else if (error == kAXErrorCannotComplete) {
                reason = @"backend_timeout";
            }
        }
        CGWindowID front = 0;
        if (focused == pid && number) front = number;
        else if (focused) {
            for (NSDictionary *entry in screen) {
                if ([entry[@"pid"] intValue] == focused) { front = [entry[@"id"] unsignedIntValue]; break; }
            }
        }
        NSMutableArray *windows = [NSMutableArray array];
        NSMutableDictionary<NSNumber *, NSDictionary *> *titles = [NSMutableDictionary dictionary];
        for (NSDictionary *entry in screen) {
            if (windows.count >= AX_MAX_WINDOWS) break;
            CGWindowID wid = [entry[@"id"] unsignedIntValue];
            pid_t owner = [entry[@"pid"] intValue];
            BOOL accessible = ax_same_user(owner);
            NSString *title = entry[@"title"];
            if (!title.length && accessible && ax_now() < deadline) {
                NSDictionary *known = titles[@(owner)];
                if (!known) {
                    NSMutableDictionary *found = [NSMutableDictionary dictionary];
                    AXError ignored = kAXErrorSuccess;
                    for (id candidate in ax_app_windows(owner, 0.25f, &ignored)) {
                        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)candidate, 0.25f);
                        CGRect bounds = ax_frame(ax_copy((__bridge AXUIElementRef)candidate, kAXPositionAttribute),
                            ax_copy((__bridge AXUIElementRef)candidate, kAXSizeAttribute));
                        CGWindowID candidateNumber = ax_window_number((__bridge AXUIElementRef)candidate, bounds, owner, screen);
                        NSString *candidateTitle = ax_text(ax_copy((__bridge AXUIElementRef)candidate, kAXTitleAttribute), AX_MAX_NAME);
                        if (candidateNumber && candidateTitle.length) found[@(candidateNumber)] = candidateTitle;
                    }
                    titles[@(owner)] = known = found;
                }
                title = known[@(wid)] ?: @"";
            }
            BOOL foreground = wid == front;
            if (!title.length && !foreground) continue;
            [windows addObject:@{@"id": @(wid), @"pid": @(owner), @"title": ax_text(title, AX_MAX_NAME),
                @"process": ax_text(entry[@"process"], AX_MAX_NAME), @"bounds": ax_rect([entry[@"frame"] rectValue]),
                @"foreground": foreground ? @YES : @NO, @"accessible": accessible ? @YES : @NO}];
        }
        BOOL web = NO;
        NSMutableArray *roots = [NSMutableArray array];
        id focus = nil;
        if (!reason && pid) {
            AXUIElementRef app = AXUIElementCreateApplication(pid);
            AXUIElementSetMessagingTimeout(app, AX_CALL_TIMEOUT);
            web = AXUIElementSetAttributeValue(app, CFSTR("AXManualAccessibility"), kCFBooleanTrue) == kAXErrorSuccess;
            focus = ax_copy(app, kAXFocusedUIElementAttribute);
            if (window) [roots addObject:window];
            if (pid == focused) {
                for (id child in ax_copy(app, kAXChildrenAttribute)) {
                    if (!ax_is_element(child)) continue;
                    AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)child, AX_CALL_TIMEOUT);
                    if ([ax_text(ax_copy((__bridge AXUIElementRef)child, kAXRoleAttribute), 64) isEqualToString:@"AXMenu"]) [roots addObject:child];
                }
                id bar = ax_copy(app, kAXMenuBarAttribute);
                if (ax_is_element(bar)) [roots addObject:bar];
            }
            CFRelease(app);
            if (!roots.count) reason = @"no_window";
        }
        NSMutableArray *nodes = [NSMutableArray array];
        NSMutableArray *store = [NSMutableArray array];
        NSMutableArray<NSString *> *labels = [NSMutableArray array];
        NSMutableArray<BuckyAxVisit *> *queue = [NSMutableArray array];
        NSMutableSet *seen = [NSMutableSet set];
        __block uint32_t visited = 0, counted = 0;
        __block BOOL stop = NO;
        void (^add)(id, int, int, uint32_t, CGRect, id, BOOL) = ^(id element, int parent, int label, uint32_t depth, CGRect clip, id owner, BOOL isRoot) {
            ++visited;
            if ([seen containsObject:element]) return;
            [seen addObject:element];
            double remaining = deadline - ax_now();
            if (remaining <= 0) { stop = YES; reason = reason ?: @"timeout"; return; }
            AXError status = kAXErrorSuccess;
            BuckyAxFacts *facts = ax_read((__bridge AXUIElementRef)element, YES, (float)fmin(AX_CALL_TIMEOUT, remaining), &status);
            if (!facts) {
                if (status == kAXErrorCannotComplete) { stop = YES; reason = @"backend_timeout"; }
                else if (isRoot && status == kAXErrorAPIDisabled) { stop = YES; reason = @"window_inaccessible"; }
                return;
            }
            NSString *role = facts.role;
            if ([role isEqualToString:@"AXColumn"]) return;
            if (focus && CFEqual((__bridge CFTypeRef)focus, (__bridge CFTypeRef)element)) facts.states |= ST_FOCUSED;
            BOOL leaf = [ax_leaf_roles() containsObject:role];
            id window = [role isEqualToString:@"AXWindow"] ? element : owner;
            CGRect frame = facts.frame;
            BOOL geometry = ax_geometry(frame);
            CGRect visible = geometry ? CGRectIntersection(frame, clip) : CGRectNull;
            if (!isRoot && (!geometry || CGRectIsEmpty(visible))) {
                if (geometry || leaf || [role isEqualToString:@"AXMenu"] || depth >= AX_MAX_DEPTH) return;
                BuckyAxVisit *visit = [BuckyAxVisit new];
                visit.children = facts.children;
                visit.window = window;
                visit.parent = parent;
                visit.label = label;
                visit.depth = depth;
                visit.clip = clip;
                [queue addObject:visit];
                return;
            }
            NSString *name = facts.name;
            uint32_t actions = facts.patterns & ~PAT_SCROLL_ITEM;
            BOOL redundant = ([role isEqualToString:@"AXStaticText"] || [role isEqualToString:@"AXImage"]) && !actions
                && name.length && label >= 0 && [labels[(NSUInteger)label] containsString:name];
            BOOL counts = isRoot || (!redundant && !(name.length == 0 && !actions && !ax_meaningful(role, facts.subrole)));
            NSMutableDictionary *node = [@{@"p": @(parent), @"r": role, @"sr": facts.subrole, @"n": name,
                @"id": facts.identifier, @"pat": @(facts.patterns), @"st": @(facts.states), @"b": ax_rect(frame)} mutableCopy];
            if (facts.value) node[@"v"] = facts.value;
            [nodes addObject:node];
            BuckyAxNode *kept = [BuckyAxNode new];
            kept.element = element;
            kept.window = window;
            kept.role = role;
            pid_t owner_pid = 0;
            AXUIElementGetPid((__bridge AXUIElementRef)element, &owner_pid);
            kept.pid = owner_pid;
            [store addObject:kept];
            int next = label;
            if (counts) {
                ++counted;
                [labels addObject:name];
                next = (int)labels.count - 1;
            }
            if (!leaf && depth < AX_MAX_DEPTH) {
                BuckyAxVisit *visit = [BuckyAxVisit new];
                visit.children = facts.children;
                visit.window = window;
                visit.parent = (int)nodes.count - 1;
                visit.label = next;
                visit.depth = depth;
                visit.clip = [ax_clip_roles() containsObject:role] ? CGRectIntersection(clip, frame)
                    : [role isEqualToString:@"AXMenu"] ? display : clip;
                [queue addObject:visit];
            }
        };
        __block NSUInteger head = 0;
        __block BOOL truncated = NO;
        void (^drain)(void) = ^{
            while (!stop && head < queue.count) {
                if (ax_now() >= deadline) { reason = @"timeout"; break; }
                if (counted >= max_nodes) { reason = @"max_nodes"; break; }
                if (visited >= AX_MAX_VISITED) { reason = @"max_visited"; break; }
                BuckyAxVisit *visit = queue[head];
                queue[head++] = (id)[NSNull null];
                NSUInteger count = MIN(visit.children.count, AX_MAX_CHILDREN);
                if (visit.children.count > count) truncated = YES;
                for (NSUInteger i = 0; i < count && !stop; i++) {
                    if (visited >= AX_MAX_VISITED) { reason = @"max_visited"; truncated = YES; break; }
                    id child = visit.children[i];
                    if (ax_is_element(child)) add(child, visit.parent, visit.label, visit.depth + 1, visit.clip, visit.window, NO);
                }
            }
        };
        for (id element in roots) {
            if (stop) break;
            add(element, -1, -1, 0, display, nil, YES);
        }
        drain();
        // A focused element outside every window (Finder's rename field
        // editor is a child of the application) is one more root.
        if (!stop && !reason && ax_is_element(focus) && ![seen containsObject:focus]) {
            AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)focus, AX_CALL_TIMEOUT);
            NSString *role = ax_text(ax_copy((__bridge AXUIElementRef)focus, kAXRoleAttribute), 64);
            if (role.length && ![@[@"AXApplication", @"AXWindow", @"AXSheet", @"AXMenuBar"] containsObject:role]) {
                add(focus, -1, -1, 0, display, nil, YES);
                drain();
            }
        }
        truncated = truncated || head < queue.count || [reason isEqualToString:@"backend_timeout"]
            || [reason isEqualToString:@"timeout"];
        uint64_t snapshot;
        os_unfair_lock_lock(&ax_lock);
        if (!ax_snapshots) ax_snapshots = [NSMutableDictionary dictionary];
        snapshot = ax_next_snapshot++;
        while (ax_snapshots.count >= AX_MAX_SNAPSHOTS) {
            NSNumber *oldest = [ax_snapshots.allKeys valueForKeyPath:@"@min.self"];
            [ax_snapshots removeObjectForKey:oldest];
        }
        ax_snapshots[@(snapshot)] = store;
        os_unfair_lock_unlock(&ax_lock);
        NSDictionary *result = @{@"root": @(number), @"root_pid": @(pid > 0 ? pid : 0), @"windows": windows,
            @"nodes": nodes, @"visited": @(visited), @"truncated": truncated ? @YES : @NO, @"reason": reason ?: @"",
            @"snapshot": @(snapshot), @"web": web ? @YES : @NO};
        int status = ax_reply(ax_json(result), json, length);
        if (status != UR_OK) {
            os_unfair_lock_lock(&ax_lock);
            [ax_snapshots removeObjectForKey:@(snapshot)];
            os_unfair_lock_unlock(&ax_lock);
        }
        return status;
    }
}

static BuckyAxNode *ax_node(uint64_t snapshot, uint32_t index) {
    BuckyAxNode *node = nil;
    os_unfair_lock_lock(&ax_lock);
    NSArray *store = ax_snapshots[@(snapshot)];
    if (index < store.count && [store[index] isKindOfClass:[BuckyAxNode class]]) node = store[index];
    os_unfair_lock_unlock(&ax_lock);
    return node;
}

/// The part of `frame` its scroll areas and web areas show, up to the
/// enclosing menu (menus are separate windows).
static CGRect ax_visible(AXUIElementRef element, CGRect frame, float timeout) {
    CGRect visible = frame;
    id current = (__bridge id)element;
    NSArray *attributes = @[(__bridge NSString *)kAXParentAttribute, (__bridge NSString *)kAXRoleAttribute,
        (__bridge NSString *)kAXPositionAttribute, (__bridge NSString *)kAXSizeAttribute];
    for (NSUInteger depth = 0; depth < AX_MAX_PARENTS && !CGRectIsEmpty(visible); depth++) {
        id parent = ax_copy((__bridge AXUIElementRef)current, kAXParentAttribute);
        if (!ax_is_element(parent)) break;
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)parent, timeout);
        CFArrayRef raw = NULL;
        if (AXUIElementCopyMultipleAttributeValues((__bridge AXUIElementRef)parent, (__bridge CFArrayRef)attributes, 0, &raw) != kAXErrorSuccess) break;
        NSArray *values = CFBridgingRelease(raw);
        NSString *role = ax_text(values[1], 64);
        if ([role isEqualToString:@"AXApplication"]) break;
        if ([ax_clip_roles() containsObject:role] || [role isEqualToString:@"AXMenu"]) {
            CGRect bounds = ax_frame(values[2], values[3]);
            if (ax_geometry(bounds)) visible = CGRectIntersection(visible, bounds);
        }
        if ([role isEqualToString:@"AXMenu"]) break;
        current = parent;
    }
    return CGRectIsNull(visible) ? CGRectZero : visible;
}

/// Whether another app's window, or another window of the same app, is
/// the one at `point`.
static BOOL ax_obscured(BuckyAxNode *node, CGPoint point, float timeout) {
    AXUIElementRef system = AXUIElementCreateSystemWide();
    AXUIElementSetMessagingTimeout(system, timeout);
    AXUIElementRef hit = NULL;
    AXError error = AXUIElementCopyElementAtPosition(system, (float)point.x, (float)point.y, &hit);
    CFRelease(system);
    if (error != kAXErrorSuccess || !hit) return YES;
    pid_t pid = 0;
    AXUIElementGetPid(hit, &pid);
    BOOL obscured = pid != node.pid;
    if (!obscured && node.window) {
        AXUIElementSetMessagingTimeout(hit, timeout);
        id window = ax_copy(hit, kAXWindowAttribute);
        obscured = ax_is_element(window) && !CFEqual((__bridge CFTypeRef)window, (__bridge CFTypeRef)node.window);
    }
    CFRelease(hit);
    return obscured;
}

static BOOL ax_focus_reached(id element, pid_t pid, float timeout) {
    AXUIElementRef system = AXUIElementCreateSystemWide();
    AXUIElementSetMessagingTimeout(system, timeout);
    id focused = ax_copy(system, kAXFocusedUIElementAttribute);
    CFRelease(system);
    pid_t owner = 0;
    if (!ax_is_element(focused) || AXUIElementGetPid((__bridge AXUIElementRef)focused, &owner) != kAXErrorSuccess || owner != pid) return NO;
    for (int depth = 0; depth < 8 && ax_is_element(focused); depth++) {
        if (CFEqual((__bridge CFTypeRef)focused, (__bridge CFTypeRef)element)) return YES;
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)focused, timeout);
        focused = ax_copy((__bridge AXUIElementRef)focused, kAXParentAttribute);
    }
    return NO;
}

/// An AXPress or attribute change (`attribute` set to `value`). An app that
/// opens a modal dialog or tracks a menu answers it only when that ends; if
/// it meanwhile answers another request, the change is running in that
/// nested loop and counts as delivered.
static int ax_deliver(BuckyAxNode *node, CFStringRef attribute, CFTypeRef value, float timeout, BOOL *pending) {
    *pending = NO;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block AXError result = kAXErrorFailure;
    id element = node.element;
    NSString *name = (__bridge NSString *)attribute;
    id argument = (__bridge id)value;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        AXUIElementRef target = (__bridge AXUIElementRef)element;
        AXUIElementSetMessagingTimeout(target, timeout);
        result = name ? AXUIElementSetAttributeValue(target, (__bridge CFStringRef)name, (__bridge CFTypeRef)argument)
                      : AXUIElementPerformAction(target, kAXPressAction);
        dispatch_semaphore_signal(done);
    });
    double started = ax_now();
    double quick = fmin(timeout, 0.5);
    if (!dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(quick * NSEC_PER_SEC)))) return ax_status(result);
    AXUIElementRef app = AXUIElementCreateApplication(node.pid);
    AXUIElementSetMessagingTimeout(app, 0.3f);
    CFTypeRef probe = NULL;
    AXError alive = AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute, &probe);
    if (probe) CFRelease(probe);
    CFRelease(app);
    if (!dispatch_semaphore_wait(done, DISPATCH_TIME_NOW)) return ax_status(result);
    if (ax_alive(alive)) {
        *pending = YES;
        return UR_OK;
    }
    double remaining = fmax(timeout - (ax_now() - started), 0);
    if (!dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(remaining * NSEC_PER_SEC)))) return ax_status(result);
    return UR_TIMEOUT;
}

/// Ops 4-7 (toggle, select, expand, collapse) and the state each one aims for.
static BOOL ax_state_reached(uint32_t op, uint32_t before, uint32_t after) {
    if (op == 4) return ((after ^ before) & (ST_CHECKED | ST_MIXED)) != 0;
    if (op == 5) return (after & ST_SELECTED) != 0;
    if (op == 6) return (after & ST_EXPANDED) != 0;
    return (after & ST_COLLAPSED) != 0;
}

int bucky_semantic_element(uint64_t snapshot, uint32_t index, uint32_t op, const uint16_t *text, size_t text_length,
                           uint32_t timeout_ms, char **json, size_t *length) {
    if (!json || !length || (text_length && !text) || op > 7) return UR_FAILED;
    *json = NULL;
    *length = 0;
    @autoreleasepool {
        BuckyAxNode *node = ax_node(snapshot, index);
        if (!node) return UR_STALE;
        if (!AXIsProcessTrusted()) return UR_UNAVAILABLE;
        float timeout = (float)fmax(timeout_ms / 1000.0, 0.05);
        float quick = fminf(timeout, AX_CALL_TIMEOUT);
        AXUIElementRef element = (__bridge AXUIElementRef)node.element;
        AXError error = kAXErrorSuccess;
        BuckyAxFacts *facts = ax_read(element, NO, quick, &error);
        if (!facts) return ax_status(error == kAXErrorNoValue ? kAXErrorInvalidUIElement : error);
        if (![facts.role isEqualToString:node.role]) return UR_STALE;
        NSMutableDictionary *reply = [NSMutableDictionary dictionary];
        if (op == 0) {
            CGRect visible = ax_geometry(facts.frame) ? ax_visible(element, facts.frame, quick) : CGRectZero;
            BOOL off = CGRectIsEmpty(visible);
            CGRect target = off ? facts.frame : visible;
            CGRect shown = CGRectIntersection(target, ax_display());
            CGRect clickable = CGRectIsEmpty(shown) ? target : shown;
            CGPoint point = CGPointMake(floor(CGRectGetMidX(clickable)), floor(CGRectGetMidY(clickable)));
            CGPoint center = CGPointMake(floor(CGRectGetMidX(target)), floor(CGRectGetMidY(target)));
            BOOL obscured = off || CGRectIsNull(target) || ax_obscured(node, point, quick);
            BOOL centerObscured = CGPointEqualToPoint(point, center) ? obscured : (off || ax_obscured(node, center, quick));
            reply[@"off"] = off ? @YES : @NO;
            reply[@"st"] = @(facts.states);
            reply[@"pat"] = @(facts.patterns);
            reply[@"b"] = ax_rect(target);
            reply[@"pt"] = @[@(point.x), @(point.y)];
            reply[@"obscured"] = obscured ? @YES : @NO;
            reply[@"center_obscured"] = centerObscured ? @YES : @NO;
        } else if (facts.states & ST_DISABLED) {
            return UR_DISABLED;
        } else if (op == 1) {
            BOOL activated = NO;
            if (ax_focused_pid() != node.pid) {
                AXUIElementRef app = AXUIElementCreateApplication(node.pid);
                AXUIElementSetMessagingTimeout(app, quick);
                activated = AXUIElementSetAttributeValue(app, kAXFrontmostAttribute, kCFBooleanTrue) == kAXErrorSuccess;
                CFRelease(app);
            }
            if (node.window && ![node.window isEqual:node.element]) {
                AXUIElementRef window = (__bridge AXUIElementRef)node.window;
                AXUIElementSetMessagingTimeout(window, quick);
                AXUIElementPerformAction(window, kAXRaiseAction);
                AXUIElementSetAttributeValue(window, kAXMainAttribute, kCFBooleanTrue);
            }
            AXUIElementSetMessagingTimeout(element, quick);
            error = AXUIElementSetAttributeValue(element, kAXFocusedAttribute, kCFBooleanTrue);
            if (error != kAXErrorSuccess) return ax_status(error);
            BOOL focused = NO;
            for (int attempt = 0; !focused && attempt < (activated ? 30 : 10); attempt++) {
                focused = ax_focus_reached(node.element, node.pid, quick);
                if (!focused) usleep(20000);
            }
            reply[@"focused"] = focused ? @YES : @NO;
        } else if (op == 2) {
            if (!(facts.patterns & PAT_INVOKE)) return UR_FAILED;
            BOOL pending = NO;
            int status = ax_deliver(node, NULL, NULL, timeout, &pending);
            if (status != UR_OK) return status;
            if (pending) {
                reply[@"pending"] = @YES;
            } else {
                AXError after = kAXErrorSuccess;
                BuckyAxFacts *current = ax_read(element, NO, quick, &after);
                reply[@"gone"] = current && [current.role isEqualToString:node.role] ? @NO : @YES;
                if (current) reply[@"st"] = @(current.states);
            }
        } else if (op == 3) {
            if (!(facts.patterns & PAT_VALUE)) return UR_FAILED;
            NSString *value = [[NSString alloc] initWithCharacters:text length:text_length];
            AXUIElementSetMessagingTimeout(element, timeout);
            error = AXUIElementSetAttributeValue(element, kAXValueAttribute, (__bridge CFStringRef)value);
            if (error != kAXErrorSuccess) return ax_status(error);
            AXUIElementSetMessagingTimeout(element, quick);
            CFTypeRef current = NULL;
            if (AXUIElementCopyAttributeValue(element, kAXValueAttribute, &current) == kAXErrorSuccess) {
                reply[@"v"] = ax_text(CFBridgingRelease(current), AX_MAX_READBACK);
            }
        } else {
            uint32_t required = op == 4 ? PAT_TOGGLE : op == 5 ? PAT_SELECTION_ITEM : PAT_EXPAND_COLLAPSE;
            if (!(facts.patterns & required)) return UR_FAILED;
            CFStringRef attribute = op == 5 ? ([facts.role isEqualToString:@"AXRadioButton"] ? NULL : kAXSelectedAttribute)
                : op >= 6 ? (__bridge CFStringRef)facts.expander : NULL;
            BOOL pending = NO;
            // AXPress flips a disclosure triangle: one already in the aimed-for
            // state is left alone.
            if (attribute || op < 6 || !ax_state_reached(op, facts.states, facts.states)) {
                int status = ax_deliver(node, attribute, op == 7 ? kCFBooleanFalse : kCFBooleanTrue, timeout, &pending);
                if (status != UR_OK) return status;
            }
            if (pending) {
                reply[@"pending"] = @YES;
            } else {
                // Apps such as Chromium apply the change asynchronously: poll
                // briefly for the aimed-for state before reading it back.
                double until = ax_now() + fmin(timeout / 2.0, 0.3);
                BOOL gone = NO;
                for (;;) {
                    AXError after = kAXErrorSuccess;
                    BuckyAxFacts *current = ax_read(element, NO, quick, &after);
                    if (!current || ![current.role isEqualToString:node.role]) {
                        gone = !current ? after == kAXErrorInvalidUIElement || after == kAXErrorNoValue : YES;
                        break;
                    }
                    reply[@"st"] = @(current.states);
                    if (ax_state_reached(op, facts.states, current.states) || ax_now() >= until) break;
                    usleep(20000);
                }
                reply[@"before"] = @(facts.states);
                reply[@"gone"] = gone ? @YES : @NO;
            }
        }
        return ax_reply(ax_json(reply), json, length);
    }
}

void bucky_semantic_retain(uint64_t snapshot, const uint32_t *indices, size_t count) {
    @autoreleasepool {
        NSMutableIndexSet *keep = [NSMutableIndexSet indexSet];
        for (size_t i = 0; indices && i < count; i++) [keep addIndex:indices[i]];
        os_unfair_lock_lock(&ax_lock);
        NSMutableArray *store = ax_snapshots[@(snapshot)];
        for (NSUInteger i = 0; i < store.count; i++) {
            if (![keep containsIndex:i]) store[i] = [NSNull null];
        }
        os_unfair_lock_unlock(&ax_lock);
    }
}

void bucky_semantic_release(uint64_t snapshot) {
    @autoreleasepool {
        os_unfair_lock_lock(&ax_lock);
        NSMutableArray *store = ax_snapshots[@(snapshot)];
        if (store) [ax_snapshots removeObjectForKey:@(snapshot)];
        os_unfair_lock_unlock(&ax_lock);
        store = nil;
    }
}
