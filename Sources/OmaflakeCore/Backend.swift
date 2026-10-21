// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// Options for `start`; `gl` only matters for QEMU windows with 3D.
public struct StartOptions: Sendable, Equatable {
    public var display: DisplayMode
    public var gl: GLBackend

    public init(display: DisplayMode, gl: GLBackend = .on) {
        self.display = display
        self.gl = gl
    }
}

/// Operations on the instances of one guest kind (plan §7.3). The CLI and the app talk only to this
/// protocol; NixOS is `QEMUBackend`, macOS is `VZBackend` on `omaf-runner` (milestone M10).
public protocol VMBackend: Sendable {
    var kind: GuestKind { get }
    /// Create the instance from an image (NixOS: qcow2 overlay; macOS: clonefile + new machine id).
    func create(name: String, recipe: String, image: ImageStore.Image, settings: RecipeDefaults) throws -> InstanceConfig
    func start(_ c: InstanceConfig, options: StartOptions) throws -> pid_t
    func stop(_ c: InstanceConfig, timeout: TimeInterval) throws -> StopResult
    func status(_ c: InstanceConfig) -> VMStatus
    /// Deletes the old state, fresh state from the same image.
    func reset(_ c: InstanceConfig) throws
    /// Deletes the old state, fresh state from another image.
    func upgrade(_ c: InstanceConfig, to image: ImageStore.Image) throws
    /// Deletes the instance folder (the share folder stays).
    func remove(_ c: InstanceConfig) throws
    func waitForSSH(_ c: InstanceConfig, timeout: TimeInterval) -> Bool
    /// Waits until the VM is up: its agent's hello (spec §11.9) or SSH; nil if it stopped or the time ran out.
    func waitForReady(_ c: InstanceConfig, timeout: TimeInterval) -> Readiness?
    /// What the VM's agent last reported; nil without one (macOS guests: not yet, spec §11.9)
    func agentState(_ c: InstanceConfig) -> AgentState?
    /// ssh invocation including `/usr/bin/ssh` in first position.
    func sshArguments(_ c: InstanceConfig, command: [String]) -> [String]
}

/// How a VM showed it is up
public enum Readiness: Sendable, Equatable {
    case agent(AgentState)
    case ssh
}

extension VMBackend {
    public func waitForReady(_ c: InstanceConfig, timeout: TimeInterval) -> Readiness? {
        c.network != .none && waitForSSH(c, timeout: timeout) ? .ssh : nil
    }

    public func agentState(_ c: InstanceConfig) -> AgentState? { nil }
}

/// NixOS guests: QEMU with HVF, qcow2 overlays (spec §6.3).
public struct QEMUBackend: VMBackend {
    public let runtime: QEMURuntime
    public let store: InstanceStore
    public var kind: GuestKind { .nixos }

    public init(runtime: QEMURuntime, store: InstanceStore) {
        self.runtime = runtime
        self.store = store
    }

    /// QEMU from config.json, VM_QEMU or PATH (see `QEMURuntime.locate`).
    public static func locate(_ paths: Paths) throws -> QEMUBackend {
        let (rt, qemuImg) = try QEMURuntime.locate(paths)
        return QEMUBackend(runtime: rt, store: InstanceStore(paths: paths, qemuImg: qemuImg))
    }

    public func create(name: String, recipe: String, image: ImageStore.Image, settings: RecipeDefaults) throws -> InstanceConfig {
        try store.create(name: name, recipe: recipe, image: image, settings: settings)
    }

    public func start(_ c: InstanceConfig, options: StartOptions) throws -> pid_t {
        try InstanceStore.checkRunsHere(c)
        return try runtime.start(c, display: options.display, gl: options.gl)
    }

    public func stop(_ c: InstanceConfig, timeout: TimeInterval) throws -> StopResult {
        try runtime.stop(c.name, timeout: timeout)
    }

    public func status(_ c: InstanceConfig) -> VMStatus { runtime.status(c.name) }

    public func reset(_ c: InstanceConfig) throws { try store.reset(c.name) }

    public func upgrade(_ c: InstanceConfig, to image: ImageStore.Image) throws {
        try InstanceStore.checkRunsHere(image)
        try store.upgrade(c.name, to: image)
    }

    public func remove(_ c: InstanceConfig) throws { try store.remove(c.name) }

    public func waitForSSH(_ c: InstanceConfig, timeout: TimeInterval) -> Bool { runtime.waitForSSH(c, timeout: timeout) }

    public func waitForReady(_ c: InstanceConfig, timeout: TimeInterval) -> Readiness? { runtime.waitForReady(c, timeout: timeout) }

    public func agentState(_ c: InstanceConfig) -> AgentState? { runtime.agentState(c.name) }

    public func sshArguments(_ c: InstanceConfig, command: [String]) -> [String] {
        SSHCommand.arguments(c, paths: store.paths, command: command)
    }
}

/// Backend per guest kind. Instances are listed by `InstanceStore` regardless of kind. A missing QEMU only
/// blocks NixOS guests and a missing omaf-runner only macOS guests; the error comes with the first use.
public struct Backends: Sendable {
    public let paths: Paths
    public let store: InstanceStore
    private let qemuResult: Result<QEMUBackend, VMError>
    private let vzResult: Result<VZBackend, VMError>

    public init(paths: Paths) throws {
        self.paths = paths
        qemuResult = Result { try QEMUBackend.locate(paths) }.mapError { $0 as? VMError ?? VMError("\($0)") }
        let qemuImg = (try? qemuResult.get())?.store.qemuImg ?? "qemu-img"
        store = InstanceStore(paths: paths, qemuImg: qemuImg)
        let store = store
        vzResult = Result { VZBackend(paths: paths, store: store, runner: try VZBackend.locateRunner(paths)) }
            .mapError { $0 as? VMError ?? VMError("\($0)") }
    }

    public var qemu: QEMUBackend { get throws { try qemuResult.get() } }
    public var vz: VZBackend { get throws { try vzResult.get() } }

    public func backend(for kind: GuestKind) throws -> any VMBackend {
        switch kind {
        case .nixos: return try qemu
        case .macos: return try vz
        }
    }

    public func backend(for c: InstanceConfig) throws -> any VMBackend { try backend(for: c.guestKind) }
}
