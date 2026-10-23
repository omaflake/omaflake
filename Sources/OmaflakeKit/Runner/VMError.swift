// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// An error with a message for people – what omaf-runner and its helpers throw. (The app's errors from the core are
/// `APIError`, with the API's `kind`.)
public struct VMError: LocalizedError, CustomStringConvertible, Sendable {
    public let description: String
    public init(_ description: String) { self.description = description }
    public var errorDescription: String? { description }
}
