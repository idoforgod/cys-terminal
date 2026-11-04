// F57-S0 fixture only. NOT compiled or executed by the preparing agent.
// Explicit file-based test keychain only; no create/default/search-list mutation.
import Foundation
import Security
import Darwin

#if BUILD_B
let build = "B"
#else
let build = "A"
#endif
func emit(_ fields: [String: Any]) {
    var row = fields
    row["epoch_seconds"] = Date().timeIntervalSince1970
    row["build"] = build
    if let data = try? JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]) {
        FileHandle.standardOutput.write(data); FileHandle.standardOutput.write(Data([10]))
    }
}
func refuse(_ reason: String) -> Never { emit(["event": "refused", "reason": reason]); exit(2) }
func value(_ name: String) -> String? {
    guard let i = CommandLine.arguments.firstIndex(of: name), i + 1 < CommandLine.arguments.count else { return nil }
    return CommandLine.arguments[i + 1]
}
func keychainPath(_ keychain: SecKeychain) -> String? {
    var length: UInt32 = 4096
    var bytes = [CChar](repeating: 0, count: Int(length))
    guard SecKeychainGetPath(keychain, &length, &bytes) == errSecSuccess else { return nil }
    return String(cString: bytes)
}
func routingSnapshot() -> (String?, [String]?) {
    var current: SecKeychain?
    let defaultStatus = SecKeychainCopyDefault(&current)
    var search: CFArray?
    let searchStatus = SecKeychainCopySearchList(&search)
    let defaultPath = defaultStatus == errSecSuccess ? current.flatMap(keychainPath) : nil
    let searchPaths: [String]?
    if searchStatus == errSecSuccess, let list = search as? [SecKeychain] {
        let paths = list.compactMap(keychainPath)
        searchPaths = paths.count == list.count ? paths : nil
    } else { searchPaths = nil }
    return (defaultPath, searchPaths)
}

guard let mode = CommandLine.arguments.dropFirst().first, ["write", "read"].contains(mode),
      let rawPath = value("--keychain"), let runID = value("--run-id"),
      let rawRoot = ProcessInfo.processInfo.environment["F57_S0_RIG_ROOT"] else {
    refuse("usage: write|read --keychain PATH --run-id ID [--no-ui]; F57_S0_RIG_ROOT required")
}
guard runID.range(of: "^[A-Za-z0-9_-]{1,64}$", options: .regularExpression) != nil else { refuse("invalid_run_id") }
let root = URL(fileURLWithPath: rawRoot).standardizedFileURL.resolvingSymlinksInPath()
let path = URL(fileURLWithPath: rawPath).standardizedFileURL.resolvingSymlinksInPath()
let rootComponents = root.pathComponents, pathComponents = path.pathComponents
guard root.path != "/", pathComponents.count > rootComponents.count,
      Array(pathComponents.prefix(rootComponents.count)) == rootComponents,
      path.lastPathComponent == "f57-s0-test.keychain-db",
      FileManager.default.fileExists(atPath: path.path),
      !path.path.contains("/Library/Keychains/"), !path.path.contains("/System/") else {
    refuse("requires_existing_dedicated_test_keychain_under_rig; login_and_system_paths_refused")
}
let before = routingSnapshot()
guard before.0 != nil, before.1 != nil else { refuse("cannot_observe_default_and_search_list") }
if let defaultPath = before.0,
   URL(fileURLWithPath: defaultPath).standardizedFileURL.resolvingSymlinksInPath() == path {
    refuse("test_target_is_default_keychain")
}
var keychain: SecKeychain?
let openStatus = path.path.withCString { SecKeychainOpen($0, &keychain) }
guard openStatus == errSecSuccess, let keychain else { emit(["event": "open_failed", "osstatus": Int(openStatus)]); exit(1) }
if CommandLine.arguments.contains("--no-ui") {
    let status = SecKeychainSetUserInteractionAllowed(false) // process-only; never changes keychain ACLs
    guard status == errSecSuccess else { emit(["event": "no_ui_failed", "osstatus": Int(status)]); exit(1) }
}
let service = "org.cys.f57s0.fixture.\(runID)"
let marker = Data("F57-S0 disposable marker \(runID)".utf8)
let start = ProcessInfo.processInfo.systemUptime
emit(["event": "operation_start", "mode": mode, "no_ui": CommandLine.arguments.contains("--no-ui")])
var result: CFTypeRef?
let status: OSStatus
if mode == "write" {
    let attrs: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service, kSecAttrAccount as String: "fixture-only",
        kSecValueData as String: marker, kSecUseKeychain as String: keychain]
    status = SecItemAdd(attrs as CFDictionary, nil)
} else {
    let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service, kSecAttrAccount as String: "fixture-only",
        kSecMatchSearchList as String: [keychain], kSecMatchLimit as String: kSecMatchLimitOne,
        kSecReturnData as String: true]
    status = SecItemCopyMatching(query as CFDictionary, &result)
}
let after = routingSnapshot()
let unchanged = before.0 == after.0 && before.1 == after.1
let matched = mode == "read" ? (result as? Data) == marker : status == errSecSuccess
emit(["event": "operation_result", "mode": mode, "osstatus": Int(status),
      "elapsed_seconds": ProcessInfo.processInfo.systemUptime - start,
      "marker_matches": matched, "default_and_search_list_unchanged": unchanged])
// Never print retrieved item bytes or passwords; never delete the fixture automatically.
exit(status == errSecSuccess && matched && unchanged ? 0 : 1)
