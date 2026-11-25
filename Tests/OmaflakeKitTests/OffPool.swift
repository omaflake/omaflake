// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Runs a test body that blocks (semaphores, socket reads, waits) on a thread of its own: blocked, it would hold one of
/// the few threads of Swift's cooperative pool that the async tests running alongside need – with the tests in parallel
/// that hung `swift test` until CI gave up (plan B176)
func offPool(_ body: @escaping @Sendable () throws -> Void) async throws {
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
        Thread.detachNewThread {
            do { try body(); c.resume() } catch { c.resume(throwing: error) }
        }
    }
}
