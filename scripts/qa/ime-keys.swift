// Test helper: select a keyboard input source and post real key events.
// Usage: ime current | ime select <source id> | ime keys <code[:cmd|:shift]>...
import Carbon
import Foundation

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

let arguments = Array(CommandLine.arguments.dropFirst())
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
