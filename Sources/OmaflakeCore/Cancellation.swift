// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// Cancels long work (image builds): `cancel()` terminates the process `Shell` is running right now and runs the
/// registered handlers (e.g. forcing a build VM off); the work checks `check()` between its steps.
/// It travels as a task-local value, so `Shell` finds it without being passed through every call:
///   `try Cancellation.$current.withValue(token) { try builder.build(…) }`
public final class Cancellation: @unchecked Sendable {
    @TaskLocal public static var current: Cancellation?

    private let lock = NSLock()
    private var cancelled = false
    private var processes: [Process] = []
    private var handlers: [@Sendable () -> Void] = []

    public init() {}

    public var isCancelled: Bool { lock.withLock { cancelled } }

    /// Stops the running process and calls the handlers; later steps see `isCancelled`.
    public func cancel() {
        let (procs, hs): ([Process], [@Sendable () -> Void]) = lock.withLock {
            guard !cancelled else { return ([], []) }
            cancelled = true
            return (processes, handlers)
        }
        for p in procs where p.isRunning { p.terminate() }
        for h in hs { h() }
    }

    /// Throws when cancelled – call it between steps.
    public func check() throws {
        if isCancelled { throw VMError("Cancelled") }
    }

    /// Runs `handler` on cancel (right away if already cancelled).
    public func onCancel(_ handler: @escaping @Sendable () -> Void) {
        let now: Bool = lock.withLock {
            if cancelled { return true }
            handlers.append(handler)
            return false
        }
        if now { handler() }
    }

    func track(_ p: Process) {
        let now: Bool = lock.withLock {
            if cancelled { return true }
            processes.append(p)
            return false
        }
        if now, p.isRunning { p.terminate() }
    }

    func untrack(_ p: Process) {
        lock.withLock { processes.removeAll { $0 === p } }
    }
}
