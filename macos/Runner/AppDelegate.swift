import Cocoa
import FlutterMacOS
import Darwin

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationWillFinishLaunching(_ notification: Notification) {
    MacStartupLog.write("native-app-launching", ProcessInfo.processInfo.operatingSystemVersionString)
    super.applicationWillFinishLaunching(notification)
  }

  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    MacStartupLog.write("native-app-reopen", "visibleWindows=\(flag)")
    // The main window hosts the ball, not a document. Never expose the nib's
    // default window while Dart is still configuring its size and appearance.
    (mainFlutterWindow as? MainFlutterWindow)?.requestBallReopen()
    return false
  }

  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }
}

enum MacStartupLog {
  private static var enginePipe: Pipe?
  private static let nativeLock = NSLock()
  private static var nativeHandle: FileHandle?
  private static var nativeDay = ""

  /// Preserve Flutter's own stderr diagnostics, including failures before Dart
  /// can create its log. Tee to the original stderr so CI keeps its console.
  static func installEngineLog() {
    guard enginePipe == nil else { return }
    do {
      let manager = FileManager.default
      let base = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
      let directory = base.appendingPathComponent("com.snappin.flutter/logs", isDirectory: true)
      try manager.createDirectory(at: directory, withIntermediateDirectories: true)
      let formatter = DateFormatter()
      formatter.dateFormat = "yyyy-MM-dd"
      let file = directory.appendingPathComponent(
        "snappin-engine-\(formatter.string(from: Date()))-\(ProcessInfo.processInfo.processIdentifier).log")
      guard manager.createFile(atPath: file.path, contents: nil) else { return }
      let output = try FileHandle(forWritingTo: file)
      let originalDescriptor = dup(STDERR_FILENO)
      guard originalDescriptor >= 0 else { return }
      let original = FileHandle(fileDescriptor: originalDescriptor, closeOnDealloc: true)
      let pipe = Pipe()
      guard dup2(pipe.fileHandleForWriting.fileDescriptor, STDERR_FILENO) >= 0 else { return }
      enginePipe = pipe
      pipe.fileHandleForReading.readabilityHandler = { handle in
        let data = handle.availableData
        guard !data.isEmpty else {
          handle.readabilityHandler = nil
          return
        }
        do { try output.write(contentsOf: data) } catch { /* Do not log recursively to stderr. */ }
        do { try original.write(contentsOf: data) } catch { /* Console may have closed. */ }
      }
      write("native-engine-log-ready", file.path)
    } catch {
      NSLog("SnapPin engine log setup failed: %@", error.localizedDescription)
    }
  }

  static func write(_ event: String, _ detail: String = "") {
    let line = "\(ISO8601DateFormatter().string(from: Date())) [\(event)] pid=\(ProcessInfo.processInfo.processIdentifier) \(detail)\n"
    NSLog("%@", line)
    nativeLock.lock()
    defer { nativeLock.unlock() }
    do {
      let formatter = DateFormatter()
      formatter.dateFormat = "yyyy-MM-dd"
      let day = formatter.string(from: Date())
      if nativeHandle == nil || nativeDay != day {
        nativeHandle?.closeFile()
        nativeHandle = nil
        let manager = FileManager.default
        let base = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let directory = base.appendingPathComponent("com.snappin.flutter/logs", isDirectory: true)
        try manager.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("snappin-native-\(day).log")
        let descriptor = Darwin.open(file.path, O_WRONLY | O_CREAT | O_APPEND, mode_t(0o600))
        guard descriptor >= 0 else {
          throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
        }
        nativeHandle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        nativeDay = day
      }
      try nativeHandle?.write(contentsOf: Data(line.utf8))
    } catch {
      NSLog("SnapPin native log failed: %@", error.localizedDescription)
    }
  }
}
