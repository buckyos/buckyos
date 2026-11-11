#include "../../src/kernel/computer_driver/src/native.m"
#include <spawn.h>
#include <signal.h>

// Self-test of the macOS Accessibility semantic backend (bucky_semantic_*).
// A child process of this binary shows a window with a button, a text field,
// a checkbox, radio buttons, a popup button, a combo box, an outline with an
// expandable row and buttons that open a modal panel, remove the first button
// or stop answering for 3 s; the parent reads and drives it only through AX
// (AX requests to one's own process run on the calling thread, which is not
// the path the Driver uses). No mouse or keyboard events are sent; focusing
// the text field activates the test window. Returns 77 without Accessibility.

extern char **environ;

@interface BuckyAxTarget : NSObject <NSOutlineViewDataSource, NSOutlineViewDelegate>
@property (nonatomic, strong) NSTextField *counter;
@property (nonatomic, strong) NSButton *pressButton;
@property (nonatomic) int presses;
@end

@implementation BuckyAxTarget
- (void)press:(id)sender {
    self.counter.stringValue = [NSString stringWithFormat:@"presses: %d", ++self.presses];
}
- (void)modal:(id)sender {
    NSPanel *panel = [[NSPanel alloc] initWithContentRect:NSMakeRect(260, 260, 220, 80)
        styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    panel.title = @"BuckyOS AX modal";
    NSTimer *timer = [NSTimer timerWithTimeInterval:1.5 repeats:NO block:^(NSTimer *t) { [NSApp stopModal]; }];
    [[NSRunLoop currentRunLoop] addTimer:timer forMode:NSModalPanelRunLoopMode];
    [NSApp runModalForWindow:panel];
    [panel orderOut:nil];
}
- (void)remove:(id)sender {
    [self.pressButton removeFromSuperview];
}
- (void)hang:(id)sender {
    [self performSelector:@selector(stall) withObject:nil afterDelay:0.05];
}
- (void)stall {
    usleep(3000000);
}
- (void)radio:(id)sender {
}
- (NSInteger)outlineView:(NSOutlineView *)view numberOfChildrenOfItem:(id)item {
    return item ? ([item isEqual:@"Bucky Folder"] ? 1 : 0) : 2;
}
- (id)outlineView:(NSOutlineView *)view child:(NSInteger)index ofItem:(id)item {
    return item ? @"Bucky Leaf" : (index ? @"Bucky Other" : @"Bucky Folder");
}
- (BOOL)outlineView:(NSOutlineView *)view isItemExpandable:(id)item {
    return [item isEqual:@"Bucky Folder"];
}
- (NSView *)outlineView:(NSOutlineView *)view viewForTableColumn:(NSTableColumn *)column item:(id)item {
    NSTableCellView *cell = [NSTableCellView new];
    NSTextField *label = [NSTextField labelWithString:item];
    label.frame = NSMakeRect(0, 0, 160, 18);
    [cell addSubview:label];
    cell.textField = label;
    return cell;
}
@end

static int target_main(void) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(200, 200, 640, 240)
        styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    window.title = @"BuckyOS AX self-test";
    BuckyAxTarget *target = [BuckyAxTarget new];
    NSButton *press = [NSButton buttonWithTitle:@"Press me" target:target action:@selector(press:)];
    press.frame = NSMakeRect(20, 190, 140, 32);
    target.pressButton = press;
    target.counter = [NSTextField labelWithString:@"presses: 0"];
    target.counter.frame = NSMakeRect(180, 196, 160, 20);
    NSTextField *field = [NSTextField textFieldWithString:@""];
    field.frame = NSMakeRect(20, 150, 340, 24);
    field.placeholderString = @"Name";
    NSButton *box = [NSButton checkboxWithTitle:@"Remember" target:nil action:nil];
    box.frame = NSMakeRect(20, 110, 200, 24);
    NSButton *modal = [NSButton buttonWithTitle:@"Open modal" target:target action:@selector(modal:)];
    modal.frame = NSMakeRect(20, 64, 140, 32);
    NSButton *remove = [NSButton buttonWithTitle:@"Remove" target:target action:@selector(remove:)];
    remove.frame = NSMakeRect(180, 64, 100, 32);
    NSButton *hang = [NSButton buttonWithTitle:@"Hang" target:target action:@selector(hang:)];
    hang.frame = NSMakeRect(20, 20, 100, 32);
    NSButton *alpha = [NSButton radioButtonWithTitle:@"Alpha" target:target action:@selector(radio:)];
    alpha.frame = NSMakeRect(400, 200, 100, 24);
    alpha.state = NSControlStateValueOn;
    NSButton *beta = [NSButton radioButtonWithTitle:@"Beta" target:target action:@selector(radio:)];
    beta.frame = NSMakeRect(510, 200, 100, 24);
    NSPopUpButton *popup = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(400, 166, 110, 26) pullsDown:NO];
    [popup addItemsWithTitles:@[@"One", @"Two"]];
    NSComboBox *combo = [[NSComboBox alloc] initWithFrame:NSMakeRect(520, 166, 100, 26)];
    [combo addItemsWithObjectValues:@[@"Red", @"Green"]];
    combo.placeholderString = @"Color";
    NSOutlineView *outline = [[NSOutlineView alloc] initWithFrame:NSMakeRect(0, 0, 220, 130)];
    NSTableColumn *column = [[NSTableColumn alloc] initWithIdentifier:@"name"];
    column.width = 200;
    [outline addTableColumn:column];
    outline.outlineTableColumn = column;
    outline.headerView = nil;
    outline.allowsMultipleSelection = YES;
    outline.dataSource = target;
    outline.delegate = target;
    NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(400, 20, 220, 130)];
    scroll.documentView = outline;
    [outline reloadData];
    for (NSView *view in @[press, target.counter, field, box, modal, remove, hang, alpha, beta, popup, combo, scroll])
        [window.contentView addSubview:view];
    [window orderFrontRegardless];
    [NSApp run];
    return 0;
}

static int failures = 0;

static void check(BOOL ok, NSString *what) {
    printf("%s %s\n", ok ? "ok  " : "FAIL", what.UTF8String);
    if (!ok) failures++;
}

static id call(int *code, int (^fn)(char **, size_t *)) {
    char *json = NULL;
    size_t length = 0;
    *code = fn(&json, &length);
    id parsed = *code == 0 && json ? [NSJSONSerialization JSONObjectWithData:[NSData dataWithBytes:json length:length] options:0 error:nil] : nil;
    free(json);
    return parsed;
}

static NSDictionary *snapshot(int *code, uint64_t window, pid_t pid, double *elapsed) {
    double started = ax_now();
    NSDictionary *tree = call(code, ^(char **json, size_t *size) {
        return bucky_semantic_snapshot(window, (uint32_t)pid, 400, 3000, json, size);
    });
    if (elapsed) *elapsed = ax_now() - started;
    return tree;
}

static NSInteger find(NSArray *nodes, NSString *role, NSString *name) {
    for (NSUInteger i = 0; i < nodes.count; i++) {
        if ([nodes[i][@"r"] isEqualToString:role] && (!name || [nodes[i][@"n"] isEqualToString:name])) return (NSInteger)i;
    }
    return -1;
}

static NSInteger row_of(NSArray *nodes, NSString *label) {
    for (NSInteger at = find(nodes, @"AXStaticText", label); at >= 0; at = [nodes[at][@"p"] integerValue]) {
        if ([nodes[at][@"r"] isEqualToString:@"AXRow"]) return at;
    }
    return -1;
}

static NSInteger child_of(NSArray *nodes, NSInteger parent, NSString *role) {
    for (NSUInteger i = 0; parent >= 0 && i < nodes.count; i++) {
        if (![nodes[i][@"r"] isEqualToString:role]) continue;
        for (NSInteger at = [nodes[i][@"p"] integerValue]; at >= 0; at = [nodes[at][@"p"] integerValue]) {
            if (at == parent) return (NSInteger)i;
        }
    }
    return -1;
}

static uint32_t bits(NSArray *nodes, NSInteger index, NSString *key) {
    return index >= 0 ? [nodes[index][key] unsignedIntValue] : 0;
}

static id element(int *code, uint64_t snap, NSInteger index, uint32_t op, NSString *text, uint32_t timeout) {
    NSMutableData *buffer = [NSMutableData dataWithLength:text.length * sizeof(unichar) + 2];
    [text getCharacters:buffer.mutableBytes range:NSMakeRange(0, text.length)];
    NSUInteger length = text.length;
    return call(code, ^(char **json, size_t *size) {
        return bucky_semantic_element(snap, (uint32_t)index, op, buffer.bytes, length, timeout, json, size);
    });
}

/// toggle, select, expand and collapse (ops 4-7): each reply carries the
/// state bits before and after, and a later snapshot agrees.
static void run_state_checks(uint64_t window, pid_t pid) {
    __block int code = 0;
    NSDictionary *tree = snapshot(&code, window, pid, NULL);
    uint64_t snap = [tree[@"snapshot"] unsignedLongLongValue];
    NSArray *nodes = tree[@"nodes"];
    NSInteger box = find(nodes, @"AXCheckBox", @"Remember"), field = find(nodes, @"AXTextField", @"Name");
    NSInteger alpha = find(nodes, @"AXRadioButton", @"Alpha"), beta = find(nodes, @"AXRadioButton", @"Beta");
    NSInteger popup = find(nodes, @"AXPopUpButton", nil), combo = find(nodes, @"AXComboBox", nil);
    NSInteger folder = row_of(nodes, @"Bucky Folder"), other = row_of(nodes, @"Bucky Other");
    NSInteger triangle = child_of(nodes, folder, @"AXDisclosureTriangle");
    check(box >= 0 && field >= 0 && alpha >= 0 && beta >= 0 && popup >= 0 && combo >= 0 && folder >= 0 && other >= 0 && triangle >= 0,
        @"radio buttons, popup, combo box, outline rows and disclosure triangle found");
    if (box < 0 || field < 0 || alpha < 0 || beta < 0 || popup < 0 || combo < 0 || folder < 0 || other < 0 || triangle < 0) {
        bucky_semantic_release(snap);
        return;
    }
    check((bits(nodes, alpha, @"st") & ST_SELECTED) && (bits(nodes, beta, @"pat") & PAT_SELECTION_ITEM), @"radio buttons show selected and offer select");
    check((bits(nodes, folder, @"pat") & (PAT_SELECTION_ITEM | PAT_EXPAND_COLLAPSE)) == (PAT_SELECTION_ITEM | PAT_EXPAND_COLLAPSE)
        && (bits(nodes, folder, @"st") & ST_COLLAPSED), @"expandable row offers select and expand (AXDisclosing) and is collapsed");
    check((bits(nodes, other, @"pat") & PAT_SELECTION_ITEM) && !(bits(nodes, other, @"pat") & PAT_EXPAND_COLLAPSE)
        && !(bits(nodes, other, @"st") & (ST_EXPANDED | ST_COLLAPSED)), @"leaf row offers select only (read-only AXDisclosing)");
    check((bits(nodes, triangle, @"pat") & PAT_EXPAND_COLLAPSE) && (bits(nodes, triangle, @"st") & ST_COLLAPSED), @"disclosure triangle offers expand");
    check((bits(nodes, combo, @"pat") & PAT_EXPAND_COLLAPSE) && (bits(nodes, combo, @"st") & ST_COLLAPSED), @"combo box offers expand (AXExpanded)");
    check(!(bits(nodes, popup, @"pat") & PAT_EXPAND_COLLAPSE), @"popup button without AXExpanded offers no expand");

    NSDictionary *(^op)(NSInteger, uint32_t) = ^NSDictionary *(NSInteger index, uint32_t command) {
        return element(&code, snap, index, command, @"", 3000);
    };
    BOOL wasChecked = (bits(nodes, box, @"st") & ST_CHECKED) != 0;
    NSDictionary *reply = op(box, 4);
    check(code == 0 && ![reply[@"gone"] boolValue] && (([reply[@"st"] unsignedIntValue] & ST_CHECKED) != 0) != wasChecked
        && (([reply[@"before"] unsignedIntValue] & ST_CHECKED) != 0) == wasChecked,
        [NSString stringWithFormat:@"toggle flips the checkbox and reads it back (code %d, %@ -> %@)", code, reply[@"before"], reply[@"st"]]);
    reply = op(beta, 5);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_SELECTED), [NSString stringWithFormat:@"select presses the radio button (code %d, st %@)", code, reply[@"st"]]);
    reply = op(other, 5);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_SELECTED), [NSString stringWithFormat:@"select sets AXSelected on a row (code %d, st %@)", code, reply[@"st"]]);
    reply = op(folder, 5);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_SELECTED), [NSString stringWithFormat:@"select another row (code %d, st %@)", code, reply[@"st"]]);
    reply = op(folder, 6);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_EXPANDED), [NSString stringWithFormat:@"expand sets AXDisclosing on the row (code %d, st %@)", code, reply[@"st"]]);
    NSDictionary *after = snapshot(&code, window, pid, NULL);
    NSArray *now = after[@"nodes"];
    check(!(bits(now, alpha, @"st") & ST_SELECTED) && (bits(now, find(now, @"AXRadioButton", @"Beta"), @"st") & ST_SELECTED),
        @"tree shows the other radio button deselected");
    check((bits(now, row_of(now, @"Bucky Folder"), @"st") & ST_SELECTED) && !(bits(now, row_of(now, @"Bucky Other"), @"st") & ST_SELECTED),
        @"row select replaces the selection");
    check(row_of(now, @"Bucky Leaf") >= 0 && (bits(now, row_of(now, @"Bucky Folder"), @"st") & ST_EXPANDED), @"expanded row shows its child");
    bucky_semantic_release([after[@"snapshot"] unsignedLongLongValue]);
    reply = op(folder, 7);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_COLLAPSED), [NSString stringWithFormat:@"collapse clears AXDisclosing (code %d, st %@)", code, reply[@"st"]]);
    reply = op(triangle, 6);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_EXPANDED), [NSString stringWithFormat:@"expand presses the disclosure triangle (code %d, st %@)", code, reply[@"st"]]);
    reply = op(triangle, 6);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_EXPANDED), [NSString stringWithFormat:@"expand of an expanded triangle leaves it expanded (code %d, st %@)", code, reply[@"st"]]);
    reply = op(triangle, 7);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_COLLAPSED), [NSString stringWithFormat:@"collapse presses the disclosure triangle (code %d, st %@)", code, reply[@"st"]]);
    reply = op(combo, 6);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_EXPANDED), [NSString stringWithFormat:@"expand sets AXExpanded on the combo box (code %d, st %@, pending %@)", code, reply[@"st"], reply[@"pending"]]);
    reply = op(combo, 7);
    check(code == 0 && ([reply[@"st"] unsignedIntValue] & ST_COLLAPSED), [NSString stringWithFormat:@"collapse clears AXExpanded on the combo box (code %d, st %@)", code, reply[@"st"]]);
    op(field, 4);
    check(code == UR_FAILED, [NSString stringWithFormat:@"toggle of a text field fails without trying (code %d)", code]);
    op(popup, 6);
    check(code == UR_FAILED, [NSString stringWithFormat:@"expand of the popup button fails without trying (code %d)", code]);
    bucky_semantic_release(snap);
}

static void run_checks(uint64_t window, pid_t pid) {
    int code = 0;
    double elapsed = 0;
    NSDictionary *tree = snapshot(&code, window, pid, &elapsed);
    check(code == 0 && tree, [NSString stringWithFormat:@"snapshot of the test window (code %d, %.0f ms, %@ nodes)",
        code, elapsed * 1000, tree ? @([tree[@"nodes"] count]) : @0]);
    if (!tree) return;
    uint64_t snap = [tree[@"snapshot"] unsignedLongLongValue];
    NSArray *nodes = tree[@"nodes"];
    check([tree[@"root"] unsignedLongLongValue] == window && [tree[@"root_pid"] intValue] == pid, @"root is the requested window");
    BOOL listed = NO;
    for (NSDictionary *entry in tree[@"windows"]) listed |= [entry[@"id"] unsignedLongLongValue] == window && [entry[@"accessible"] boolValue];
    check(listed, @"window list contains the test window as accessible");
    NSInteger pressIndex = find(nodes, @"AXButton", @"Press me");
    NSInteger fieldIndex = find(nodes, @"AXTextField", @"Name");
    NSInteger boxIndex = find(nodes, @"AXCheckBox", @"Remember");
    NSInteger modalIndex = find(nodes, @"AXButton", @"Open modal");
    NSInteger removeIndex = find(nodes, @"AXButton", @"Remove");
    NSInteger hangIndex = find(nodes, @"AXButton", @"Hang");
    check(pressIndex >= 0 && fieldIndex >= 0 && boxIndex >= 0 && modalIndex >= 0 && removeIndex >= 0 && hangIndex >= 0,
        @"buttons, text field (named by its placeholder) and checkbox found");
    if (pressIndex < 0 || fieldIndex < 0 || boxIndex < 0 || modalIndex < 0 || removeIndex < 0 || hangIndex < 0) return;
    check(find(nodes, @"AXStaticText", @"presses: 0") >= 0, @"static text is named by its value");
    check(([nodes[pressIndex][@"pat"] unsignedIntValue] & PAT_INVOKE) != 0, @"button offers invoke");
    check(([nodes[fieldIndex][@"pat"] unsignedIntValue] & (PAT_VALUE | PAT_FOCUSABLE)) == (PAT_VALUE | PAT_FOCUSABLE),
        @"text field offers set_value and focus");
    check(([nodes[boxIndex][@"pat"] unsignedIntValue] & PAT_TOGGLE) != 0, @"checkbox offers toggle");

    NSDictionary *resolved = element(&code, snap, pressIndex, 0, @"", 3000);
    NSArray *point = resolved[@"pt"], *bounds = nodes[pressIndex][@"b"];
    BOOL inside = point.count == 2 && bounds.count == 4
        && [point[0] doubleValue] >= [bounds[0] doubleValue] && [point[0] doubleValue] <= [bounds[0] doubleValue] + [bounds[2] doubleValue]
        && [point[1] doubleValue] >= [bounds[1] doubleValue] && [point[1] doubleValue] <= [bounds[1] doubleValue] + [bounds[3] doubleValue];
    check(code == 0 && inside && ![resolved[@"off"] boolValue],
        [NSString stringWithFormat:@"resolve gives a point on the button (%@,%@; obscured %@)", point.firstObject, point.lastObject, resolved[@"obscured"]]);

    NSDictionary *pressed = element(&code, snap, pressIndex, 2, @"", 3000);
    usleep(100000);
    NSDictionary *after = snapshot(&code, window, pid, NULL);
    check(pressed && find(after[@"nodes"], @"AXStaticText", @"presses: 1") >= 0, @"invoke presses the button");
    bucky_semantic_release([after[@"snapshot"] unsignedLongLongValue]);

    NSString *text = @"语义 AX ✓";
    NSDictionary *set = element(&code, snap, fieldIndex, 3, text, 3000);
    check(code == 0 && [set[@"v"] isEqualToString:text], [NSString stringWithFormat:@"set_value writes Unicode text and reads it back (code %d)", code]);

    element(&code, snap, boxIndex, 2, @"", 3000);
    usleep(100000);
    after = snapshot(&code, window, pid, NULL);
    NSInteger box = find(after[@"nodes"], @"AXCheckBox", @"Remember");
    check(box >= 0 && ([after[@"nodes"][box][@"st"] unsignedIntValue] & ST_CHECKED), @"invoke toggles the checkbox and the tree shows checked");
    bucky_semantic_release([after[@"snapshot"] unsignedLongLongValue]);

    run_state_checks(window, pid);

    NSDictionary *focused = element(&code, snap, fieldIndex, 1, @"", 3000);
    check(code == 0 && [focused[@"focused"] boolValue], [NSString stringWithFormat:@"focus activates the window and focuses the field (code %d)", code]);

    double started = ax_now();
    NSDictionary *modal = element(&code, snap, modalIndex, 2, @"", 3000);
    elapsed = ax_now() - started;
    check(code == 0 && [modal[@"pending"] boolValue] && elapsed < 1.4,
        [NSString stringWithFormat:@"invoke that opens a modal returns as delivered (code %d, %.0f ms)", code, elapsed * 1000]);
    usleep(1700000);

    element(&code, snap, removeIndex, 2, @"", 3000);
    usleep(150000);
    element(&code, snap, pressIndex, 2, @"", 3000);
    check(code == UR_STALE, [NSString stringWithFormat:@"removed button is stale (code %d)", code]);
    bucky_semantic_release(snap);
    element(&code, snap, fieldIndex, 0, @"", 3000);
    check(code == UR_STALE, @"released snapshot is stale");

    after = snapshot(&code, window, pid, NULL);
    hangIndex = find(after[@"nodes"], @"AXButton", @"Hang");
    element(&code, [after[@"snapshot"] unsignedLongLongValue], hangIndex, 2, @"", 3000);
    bucky_semantic_release([after[@"snapshot"] unsignedLongLongValue]);
    usleep(200000);
    NSDictionary *hung = snapshot(&code, window, pid, &elapsed);
    check(code == 0 && [hung[@"reason"] isEqualToString:@"backend_timeout"] && [hung[@"truncated"] boolValue] && elapsed < 1.6,
        [NSString stringWithFormat:@"unresponsive app returns backend_timeout (code %d, %@, %.0f ms)", code, hung[@"reason"], elapsed * 1000]);
    bucky_semantic_release([hung[@"snapshot"] unsignedLongLongValue]);
}

int main(int argc, char **argv) {
    @autoreleasepool {
        setvbuf(stdout, NULL, _IONBF, 0);
        if (argc > 1 && !strcmp(argv[1], "--target")) return target_main();
        if (bucky_semantic_status() != 0) {
            puts("Accessibility is not granted to the responsible app");
            return 77;
        }
        pid_t child = 0;
        char *args[] = {argv[0], "--target", NULL};
        if (posix_spawn(&child, argv[0], NULL, NULL, args, environ)) return 1;
        uint64_t window = 0;
        for (int attempt = 0; !window && attempt < 100; attempt++) {
            usleep(50000);
            for (NSDictionary *entry in ax_screen_windows(ax_display())) {
                if ([entry[@"pid"] intValue] == child) window = [entry[@"id"] unsignedLongLongValue];
            }
        }
        if (window) {
            usleep(300000);
            run_checks(window, child);
        } else {
            check(NO, @"test window appeared");
        }
        kill(child, SIGTERM);
        waitpid(child, NULL, 0);
        printf("%s\n", failures ? "AX self-test failed" : "AX self-test passed");
        return failures ? 1 : 0;
    }
}
