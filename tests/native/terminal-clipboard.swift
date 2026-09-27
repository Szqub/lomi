import AppKit
import Foundation

// Preserve every clipboard representation without logging or retaining its contents.
let board = NSPasteboard.general
let mode = CommandLine.arguments[1]
let directory = URL(fileURLWithPath: CommandLine.arguments[2])
let backup = directory.appendingPathComponent("clipboard-backup.plist")
let marker = NSPasteboard.PasteboardType("dev.lomi.clipboard-smoke")
switch mode {
case "backup":
    let items = (board.pasteboardItems ?? []).map { item in
        Dictionary(uniqueKeysWithValues: item.types.compactMap { type in
            item.data(forType: type).map { (type.rawValue, $0) }
        })
    }
    let data = try PropertyListSerialization.data(fromPropertyList: items, format: .binary, options: 0)
    try data.write(to: backup, options: .atomic)
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: backup.path)
case "restore":
    defer { try? FileManager.default.removeItem(at: backup) }
    let markerOwned = board.string(forType: marker) == directory.path
    let expectedChangeCount = CommandLine.arguments.count > 3
        ? Int(CommandLine.arguments[3])
        : nil
    let unchangedSinceSmokeAction = expectedChangeCount == board.changeCount
    if markerOwned || unchangedSinceSmokeAction {
        let data = try Data(contentsOf: backup)
        let items = try PropertyListSerialization.propertyList(from: data, format: nil) as! [[String: Data]]
        board.clearContents()
        board.writeObjects(items.map { values in
            let item = NSPasteboardItem()
            for (type, bytes) in values { item.setData(bytes, forType: NSPasteboard.PasteboardType(type)) }
            return item
        })
        print("restored")
    } else {
        print("preserved-newer-clipboard")
    }
case "status":
    let status: [String: Any] = [
        "markerOwned": board.string(forType: marker) == directory.path,
        "changeCount": board.changeCount,
    ]
    let data = try JSONSerialization.data(withJSONObject: status, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
case "activate":
    guard CommandLine.arguments.count > 3,
          let pid = Int32(CommandLine.arguments[3])
    else {
        print("not-registered")
        exit(2)
    }
    guard let application = NSRunningApplication(processIdentifier: pid) else {
        print("not-registered")
        exit(2)
    }
    let activated = application.activate(options: [.activateAllWindows])
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
    let frontmostPid = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1
    print(
        "activation-requested=\(activated); active=\(application.isActive); frontmostPid=\(frontmostPid)"
    )
case "image", "image-only", "text":
    board.clearContents()
    board.setString(directory.path, forType: marker)
    if mode != "text" {
        let png = try Data(contentsOf: directory.appendingPathComponent("fixture.png"))
        board.setData(png, forType: .png)
        if let tiff = NSBitmapImageRep(data: png)?.tiffRepresentation { board.setData(tiff, forType: .tiff) }
        if mode == "image" { board.setString("https://example.test/fixture.png", forType: .string) }
    } else {
        board.setString("zażółć 🦀\r\nsecond line", forType: .string)
    }
default:
    fatalError("Unknown clipboard fixture mode")
}
