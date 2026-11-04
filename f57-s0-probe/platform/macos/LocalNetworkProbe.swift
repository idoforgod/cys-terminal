// F57-S0 disposable Local Network attribution model; not the iroh transport.
// Compile/run only on the isolated spare user named in procedures/MACOS-LOCAL-NETWORK.md.
import Foundation
import Darwin
func emit(_ fields: [String: Any]) {
    var row = fields; row["epoch_seconds"] = Date().timeIntervalSince1970
    row["uptime_seconds"] = ProcessInfo.processInfo.systemUptime
    if let data = try? JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]) {
        FileHandle.standardOutput.write(data); FileHandle.standardOutput.write(Data([10]))
    }
}
func value(_ name: String) -> String? {
    guard let i = CommandLine.arguments.firstIndex(of: name), i + 1 < CommandLine.arguments.count else { return nil }
    return CommandLine.arguments[i + 1]
}
let mode = value("--mode") ?? "browse"
let seconds = Double(value("--seconds") ?? "180") ?? 0
guard ["browse", "publish"].contains(mode), seconds >= 1, seconds <= 3600 else { emit(["event": "invalid_arguments"]); exit(2) }
final class Probe: NSObject, NetServiceBrowserDelegate, NetServiceDelegate {
    let browser = NetServiceBrowser()
    let service = NetService(domain: "local.", type: "_f57probe._tcp.", name: UUID().uuidString, port: 9)
    var found = 0
    func start() {
        if mode == "browse" {
            browser.delegate = self; browser.searchForServices(ofType: "_f57probe._tcp.", inDomain: "local.")
        } else { service.delegate = self; service.publish() }
        emit(["event": "attempt", "mode": mode])
    }
    func retry() { browser.stop(); service.stop(); start() }
    func netServiceBrowserWillSearch(_ browser: NetServiceBrowser) { emit(["event": "browser_started"]) }
    func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
        emit(["event": "browser_error", "error": errorDict.mapValues { $0.intValue }])
    }
    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        found += 1; emit(["event": "service_observed", "count": found]) // no peer names or hostnames
    }
    func netServiceDidPublish(_ sender: NetService) { emit(["event": "published"]) }
    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        emit(["event": "publish_error", "error": errorDict.mapValues { $0.intValue }])
    }
}
let probe = Probe(); probe.start()
let retryTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in probe.retry() }
RunLoop.current.run(until: Date(timeIntervalSinceNow: seconds))
retryTimer.invalidate(); probe.browser.stop(); probe.service.stop()
emit(["event": "finished", "observed_services": probe.found, "verdict": "UNMEASURED_OPERATOR_OBSERVATION_REQUIRED"])
