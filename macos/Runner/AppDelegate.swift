import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationWillFinishLaunching(_ notification: Notification) {
    MacStartupLog.write("native-app-launching", ProcessInfo.processInfo.operatingSystemVersionString)
    super.applicationWillFinishLaunching(notification)
  }

  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    MacStartupLog.write("native-app-reopen", "visibleWindows=\(flag)")
    if let window = mainFlutterWindow {
      window.deminiaturize(nil)
      window.makeKeyAndOrderFront(nil)
      sender.activate(ignoringOtherApps: true)
    }
    return true
  }

  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }
}

enum MacStartupLog {
  static func write(_ event: String, _ detail: String = "") {
    let line = "\(ISO8601DateFormatter().string(from: Date())) [\(event)] pid=\(ProcessInfo.processInfo.processIdentifier) \(detail)\n"
    NSLog("%@", line)
    do {
      let manager = FileManager.default
      let base = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
      let directory = base.appendingPathComponent("com.snappin.flutter/logs", isDirectory: true)
      try manager.createDirectory(at: directory, withIntermediateDirectories: true)
      let formatter = DateFormatter()
      formatter.dateFormat = "yyyy-MM-dd"
      let file = directory.appendingPathComponent("snappin-native-\(formatter.string(from: Date())).log")
      if !manager.fileExists(atPath: file.path) {
        manager.createFile(atPath: file.path, contents: nil)
      }
      let handle = try FileHandle(forWritingTo: file)
      defer { handle.closeFile() }
      handle.seekToEndOfFile()
      handle.write(Data(line.utf8))
    } catch {
      NSLog("SnapPin native log failed: %@", error.localizedDescription)
    }
  }
}
