// Test helper: select a keyboard input source and post real key events.
// Usage: ime current | ime select <source id> | ime keys <code[:cmd|:shift]>...
import Carbon
import Foundation
import AppKit
import ApplicationServices

func currentSource() -> String {
    let source = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
    let id = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
    return Unmanaged<CFString>.fromOpaque(id!).takeUnretainedValue() as String
}

func select(_ id: String) -> Bool {
    let filter = [kTISPropertyInputSourceID as String: id] as CFDictionary
    guard let list = TISCreateInputSourceList(filter, false)?.takeRetainedValue() as? [TISInputSource],
          let source = list.first else { return false }
    return TISSelectInputSource(source) == noErr
}

// Press modifier keys as real key events, like a person does, so the
// input method sees Command or Shift held down before the key.
let modifierKeys: [String: (CGKeyCode, CGEventFlags)] = ["cmd": (55, .maskCommand), "shift": (56, .maskShift)]

func post(_ code: CGKeyCode, down: Bool, flags: CGEventFlags) {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
    event.flags = flags
    event.post(tap: .cghidEventTap)
    usleep(50_000)
}

func press(_ code: CGKeyCode, modifiers: [String]) {
    var flags: CGEventFlags = []
    for name in modifiers {
        let (key, flag) = modifierKeys[name]!
        flags.insert(flag)
        post(key, down: true, flags: flags)
    }
    post(code, down: true, flags: flags)
    post(code, down: false, flags: flags)
    for name in modifiers.reversed() {
        let (key, flag) = modifierKeys[name]!
        flags.remove(flag)
        post(key, down: false, flags: flags)
    }
    usleep(120_000)
}


// AX traversal limits protect this test helper from large or cyclic trees.
let axMaxDepth = 48
let axMaxNodes = 12_000
let axSearchSeconds = 8.0

func fail(_ code: String, _ message: String) -> Never {
    let payload = try! JSONSerialization.data(withJSONObject: ["code": code, "message": message])
    FileHandle.standardError.write(payload)
    FileHandle.standardError.write(Data("\n".utf8))
    exit(1)
}

func json(_ value: Any) {
    // All callers supply JSON primitives or dictionaries built here.
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .fragmentsAllowed])
    print(String(decoding: data, as: UTF8.self))
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var result: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, name as CFString, &result)
    if error == .success { return result }
    if error == .attributeUnsupported || error == .noValue { return nil }
    fail("AX_READ_FAILED", "Cannot read \(name): AX error \(error.rawValue)")
}

func axElement(_ value: CFTypeRef?) -> AXUIElement? {
    guard let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
    return (value as! AXUIElement)
}

func appElement(_ pid: pid_t) -> AXUIElement {
    guard AXIsProcessTrusted() else {
        fail("AX_PERMISSION", "Accessibility permission is missing. Enable it for the terminal or runner in System Settings.")
    }
    guard kill(pid, 0) == 0 else { fail("PROCESS_MISSING", "App process \(pid) is not running") }
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 0.5)
    return app
}

func walk(_ root: AXUIElement, visit: (AXUIElement) -> Bool) {
    var stack: [(AXUIElement, Int)] = [(root, 0)]
    var count = 0
    let started = ProcessInfo.processInfo.systemUptime
    while let (element, depth) = stack.popLast() {
        count += 1
        if count > axMaxNodes || ProcessInfo.processInfo.systemUptime - started > axSearchSeconds {
            fail("AX_LIMIT", "Accessibility search exceeded its node or time limit")
        }
        if visit(element) { return }
        let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
        if depth >= axMaxDepth && !children.isEmpty {
            fail("AX_LIMIT", "Accessibility search exceeded depth \(axMaxDepth)")
        }
        if stack.count + count + children.count > axMaxNodes {
            fail("AX_LIMIT", "Accessibility tree exceeds \(axMaxNodes) nodes")
        }
        for child in children.reversed() { stack.append((child, depth + 1)) }
    }
}

func find(_ app: AXUIElement, id: String) -> AXUIElement {
    var found: AXUIElement?
    walk(app) { element in
        if attribute(element, "AXDOMIdentifier") as? String == id {
            found = element
            return true
        }
        return false
    }
    guard let found else { fail("AX_NOT_FOUND", "No Accessibility element with DOM id \(id)") }
    return found
}

func findButton(_ app: AXUIElement, label: String) -> AXUIElement {
    var matches: [AXUIElement] = []
    walk(app) { element in
        if attribute(element, kAXRoleAttribute) as? String == kAXButtonRole {
            let labels = [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute]
                .compactMap { attribute(element, $0) as? String }
            if labels.contains(label) { matches.append(element) }
        }
        return false
    }
    guard matches.count == 1, let found = matches.first else {
        fail("AX_BUTTON_MATCH", "Expected one button labelled \(label); found \(matches.count)")
    }
    return found
}

func textValue(_ element: AXUIElement) -> Any {
    if let value = attribute(element, kAXValueAttribute) {
        if let string = value as? String { return string }
        if let number = value as? NSNumber { return number }
    }
    // Static HTML labels may expose their text as child AXStaticText nodes.
    var pieces: [String] = []
    walk(element) { child in
        if attribute(child, kAXRoleAttribute) as? String == kAXStaticTextRole,
           let value = attribute(child, kAXValueAttribute) as? String, !value.isEmpty {
            pieces.append(value)
        }
        return false
    }
    return pieces.isEmpty ? NSNull() : pieces.joined()
}

func frame(_ element: AXUIElement) -> CGRect? {
    guard let position = attribute(element, kAXPositionAttribute),
          let size = attribute(element, kAXSizeAttribute),
          CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero
    var extent = CGSize.zero
    guard AXValueGetValue(position as! AXValue, .cgPoint, &point),
          AXValueGetValue(size as! AXValue, .cgSize, &extent) else { return nil }
    return CGRect(origin: point, size: extent)
}

func description(_ element: AXUIElement) -> [String: Any] {
    let rect = frame(element)
    let bounds: Any = rect.map { ["x": $0.minX, "y": $0.minY, "width": $0.width, "height": $0.height] } ?? NSNull()
    return ["domId": attribute(element, "AXDOMIdentifier") as? String ?? "",
            "role": attribute(element, kAXRoleAttribute) as? String ?? "",
            "frame": bounds, "value": textValue(element),
            "focused": attribute(element, kAXFocusedAttribute) as? Bool ?? false,
            "enabled": attribute(element, kAXEnabledAttribute) as? Bool ?? true]
}

func sessionUnlocked() -> Bool? {
    guard let session = CGSessionCopyCurrentDictionary() as? [String: Any],
          let console = session[kCGSessionOnConsoleKey as String] as? Bool else { return nil }
    let locked = session["CGSSessionScreenIsLocked"] as? Bool ?? false
    return console && !locked
}

func guardFrontmost(_ pid: pid_t) {
    guard sessionUnlocked() == true else { fail("SCREEN_LOCKED", "Screen is locked or session state is unavailable") }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        fail("FOCUS_CHANGED", "The target app is not frontmost; stopped before posting events")
    }
}

func click(_ element: AXUIElement, pid: pid_t) {
    guardFrontmost(pid)
    guard attribute(element, kAXEnabledAttribute) as? Bool != false,
          let rect = frame(element), rect.width > 0, rect.height > 0,
          rect.minX.isFinite, rect.minY.isFinite, rect.width.isFinite, rect.height.isFinite else {
        fail("AX_FRAME", "Target is disabled or has no usable frame")
    }
    let point = CGPoint(x: rect.midX, y: rect.midY)
    guard NSScreen.screens.contains(where: { screen in
        // AX/CG coordinates start at the upper left of the main screen.
        let top = NSScreen.screens.first?.frame.maxY ?? 0
        let bounds = CGRect(x: screen.frame.minX, y: top - screen.frame.maxY,
                            width: screen.frame.width, height: screen.frame.height)
        return bounds.contains(point)
    }) else { fail("AX_OFFSCREEN", "Target centre is outside the screens") }
    for type in [CGEventType.leftMouseDown, .leftMouseUp] {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            fail("EVENT_FAILED", "Cannot create mouse event")
        }
        event.post(tap: .cghidEventTap)
        usleep(50_000)
    }
}

func focused(_ app: AXUIElement) -> AXUIElement {
    if let element = axElement(attribute(app, kAXFocusedUIElementAttribute)) { return element }
    var found: AXUIElement?
    walk(app) { element in
        if attribute(element, kAXFocusedAttribute) as? Bool == true {
            found = element
            return true
        }
        return false
    }
    guard let found else { fail("AX_NO_FOCUS", "App has no focused Accessibility element") }
    return found
}

func nativeCommand(_ args: [String]) -> Bool {
    guard let command = args.first else { return false }
    if command == "preflight" {
        guard args.count == 1 else { fail("ARGUMENT", "preflight takes no arguments") }
        let filter = [kTISPropertyInputSourceID as String: "com.apple.inputmethod.Korean.2SetKorean"] as CFDictionary
        let sources = TISCreateInputSourceList(filter, false)?.takeRetainedValue() as? [TISInputSource] ?? []
        let apps = NSWorkspace.shared.runningApplications.filter {
            $0.localizedName == "HWP Workbench" || $0.executableURL?.lastPathComponent == "HWP Workbench" ||
            $0.bundleIdentifier == "io.github.jonathanjie.hwp-workbench"
        }.map { ["pid": Int($0.processIdentifier), "bundle": $0.bundleURL?.path ?? ""] as [String: Any] }
        json(["screenUnlocked": sessionUnlocked().map { $0 as Any } ?? NSNull(),
              "trusted": AXIsProcessTrusted(), "koreanAvailable": !sources.isEmpty,
              "currentSource": currentSource(), "instances": apps])
        return true
    }
    let commands = ["ax-find", "ax-click", "ax-value", "ax-focused", "ax-processes", "ax-click-text", "ax-find-text", "wait-window", "guarded-keys"]
    guard commands.contains(command) else { return false }
    guard args.count >= 2, let pid = Int32(args[1]), pid > 0 else { fail("ARGUMENT", "\(command) requires a positive app PID") }
    let app = appElement(pid)
    switch command {
    case "ax-focused":
        guard args.count == 2 else { fail("ARGUMENT", "ax-focused <pid>") }
        json(description(focused(app)))
    case "ax-processes":
        guard args.count == 2 else { fail("ARGUMENT", "ax-processes <pid>") }
        // Remote WebKit AX elements can identify the WebContent process even
        // when launchd is its Unix parent. Only this app's tree is traversed.
        var pids: Set<pid_t> = [pid]
        walk(app) { element in
            var owner: pid_t = 0
            if AXUIElementGetPid(element, &owner) == .success && owner > 0 {
                pids.insert(owner)
            }
            return false
        }
        json(pids.sorted().map(Int.init))
    case "wait-window":
        guard args.count == 3, let seconds = Double(args[2]), seconds > 0, seconds <= 120 else {
            fail("ARGUMENT", "wait-window <pid> <seconds: 0..120>")
        }
        let start = ProcessInfo.processInfo.systemUptime
        repeat {
            let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
            if let window = windows.first(where: {
                ($0[kCGWindowOwnerPID as String] as? Int32) == pid &&
                ($0[kCGWindowLayer as String] as? Int) == 0 &&
                (($0[kCGWindowBounds as String] as? [String: Any])?["Width"] as? Double ?? 0) > 0 &&
                (($0[kCGWindowBounds as String] as? [String: Any])?["Height"] as? Double ?? 0) > 0
            }) {
                json(["id": window[kCGWindowNumber as String] ?? NSNull(), "seconds": ProcessInfo.processInfo.systemUptime - start])
                return true
            }
            usleep(100_000)
        } while ProcessInfo.processInfo.systemUptime - start < seconds
        fail("WINDOW_TIMEOUT", "No on-screen window for app \(pid) within \(seconds) seconds")
    case "guarded-keys":
        guard args.count >= 4 else { fail("ARGUMENT", "guarded-keys <pid> <dom-id or -> <code[:modifier]>...") }
        for item in args.dropFirst(3) {
            let parts = item.split(separator: ":").map(String.init)
            guard let first = parts.first, let code = CGKeyCode(first), code <= 127,
                  parts.dropFirst().allSatisfy({ modifierKeys[$0] != nil }) else { fail("ARGUMENT", "Invalid key: \(item)") }
            guardFrontmost(pid)
            if args[2] != "-" && attribute(focused(app), "AXDOMIdentifier") as? String != args[2] {
                fail("FOCUS_CHANGED", "Focus left \(args[2]); stopped before posting keys")
            }
            press(code, modifiers: Array(parts.dropFirst()))
        }
        json(["sent": args.count - 3])
    default:
        guard args.count == 3 else { fail("ARGUMENT", "\(command) <pid> <DOM id or button label>") }
        let element = command.hasSuffix("-text") ? findButton(app, label: args[2]) : find(app, id: args[2])
        if command == "ax-value" {
            json(textValue(element))
        } else {
            // A click can remove its target (for example, the Save sheet).
            // Read the snapshot before posting events, not after UI mutation.
            var snapshot = description(element)
            if command == "ax-click" || command == "ax-click-text" {
                click(element, pid: pid)
                snapshot["clicked"] = true
            }
            json(snapshot)
        }
    }
    return true
}

let arguments = Array(CommandLine.arguments.dropFirst())
if nativeCommand(arguments) { exit(0) }
switch arguments.first {
case "current":
    print(currentSource())
case "select":
    print(select(arguments[1]) ? "selected \(currentSource())" : "not found")
case "keys":
    for item in arguments.dropFirst() {
        let parts = item.split(separator: ":").map(String.init)
        press(CGKeyCode(parts[0])!, modifiers: Array(parts.dropFirst()))
    }
    print("sent \(arguments.count - 1)")
default:
    print("usage: ime current | select <id> | keys <code>...")
}
