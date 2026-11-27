// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import FSKit

// omaflake's file system for a macOS guest's volumes (plan B190): an FSKit module that shows a folder – the hidden
// virtio-fs mount of a volume – as a volume of its own, so the Finder lists it under Locations like an external disk
// (a virtio-fs mount is neither local nor a server's, plan B189). Every operation goes through to the folder as it
// is: no cache of its own, names, sizes, modes and times are the folder's.
//
// Started like Apple's own FSKit modules (checked in a macOS 27 guest): the entry point is `_NSExtensionMain` (linker
// `-e`, Package.swift) and `VolumeFileSystem` the principal class (`EXExtensionPrincipalClass`). A Swift `@main`
// AppExtension trapped in ExtensionFoundation; a `main` of its own calling NSExtensionMain recursed.
//
// The volume protocols of FSKit V1 (`FSVolume.Operations` …) are deprecated in macOS 27 in favour of the `…Handler`
// ones; they work (checked) – OPEN: moving to the handlers.

@objc(VolumeFileSystem)
final class VolumeFileSystem: FSUnaryFileSystem, FSUnaryFileSystemOperations, @unchecked Sendable {
    func probeResource(resource: FSResource, replyHandler: @escaping (FSProbeResult?, (any Error)?) -> Void) {
        guard let r = resource as? FSPathURLResource else {
            return replyHandler(.notRecognized, nil)
        }
        replyHandler(.usable(name: r.url.lastPathComponent, containerID: FSContainerIdentifier(uuid: UUID())), nil)
    }

    func loadResource(resource: FSResource, options: FSTaskOptions,
                      replyHandler: @escaping (FSVolume?, (any Error)?) -> Void) {
        guard let r = resource as? FSPathURLResource else {
            return replyHandler(nil, POSIXError(.EINVAL))
        }
        containerStatus = .ready
        replyHandler(FolderVolume(root: r.url), nil)
    }

    func unloadResource(resource: FSResource, options: FSTaskOptions, replyHandler: @escaping ((any Error)?) -> Void) {
        replyHandler(nil)
    }
}

/// An item of the volume: its path in the folder (it changes when the item or a folder above it is renamed)
final class FolderItem: FSItem, @unchecked Sendable {
    var path: String
    let id: UInt64

    init(path: String, id: UInt64) {
        self.path = path
        self.id = id
    }
}

final class FolderVolume: FSVolume, FSVolume.Operations, FSVolume.ReadWriteOperations, @unchecked Sendable {
    let rootPath: String
    /// The items FSKit holds, by their file id (the folder's inode; the root has FSKit's own id)
    private var items: [UInt64: FolderItem] = [:]
    private let lock = NSLock()

    init(root: URL) {
        rootPath = root.path
        super.init(volumeID: FSVolume.Identifier(uuid: UUID()), volumeName: FSFileName(string: root.lastPathComponent))
    }

    // MARK: Helpers

    private func item(_ path: String) -> FolderItem? {
        var st = stat()
        guard lstat(path, &st) == 0 else { return nil }
        let id = path == rootPath ? FSItem.Identifier.rootDirectory.rawValue : UInt64(st.st_ino)
        lock.lock()
        defer { lock.unlock() }
        if let known = items[id], known.path == path { return known }
        let made = FolderItem(path: path, id: id)
        items[id] = made
        return made
    }

    private func parentID(of path: String) -> UInt64 {
        path == rootPath
            ? FSItem.Identifier.rootDirectory.rawValue
            : item((path as NSString).deletingLastPathComponent)?.id ?? FSItem.Identifier.rootDirectory.rawValue
    }

    private static func itemType(_ mode: mode_t) -> FSItem.ItemType {
        switch mode & S_IFMT {
        case S_IFDIR: .directory
        case S_IFLNK: .symlink
        default: .file
        }
    }

    private func attributes(_ path: String, id: UInt64, parent: UInt64) -> FSItem.Attributes? {
        var st = stat()
        guard lstat(path, &st) == 0 else { return nil }
        let a = FSItem.Attributes()
        a.type = Self.itemType(st.st_mode)
        a.mode = UInt32(st.st_mode & 0o7777)
        a.linkCount = UInt32(st.st_nlink)
        a.uid = st.st_uid
        a.gid = st.st_gid
        a.size = UInt64(st.st_size)
        a.allocSize = UInt64(st.st_blocks) * 512
        a.fileID = FSItem.Identifier(rawValue: id) ?? .invalid
        a.parentID = FSItem.Identifier(rawValue: parent) ?? .invalid
        a.flags = UInt32(st.st_flags)
        a.accessTime = st.st_atimespec
        a.modifyTime = st.st_mtimespec
        a.changeTime = st.st_ctimespec
        a.birthTime = st.st_birthtimespec
        return a
    }

    private func lastError() -> POSIXError { POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }

    /// What this sandboxed extension makes carries com.apple.quarantine, which virtio-fs takes to the host's folder
    /// (checked); a volume's files are the user's, not downloads
    private func unquarantine(_ path: String) {
        removexattr(path, "com.apple.quarantine", XATTR_NOFOLLOW)
    }

    /// Size, mode and times of a set-attributes request on `path`; owners stay the host's
    private func apply(_ a: FSItem.SetAttributesRequest, to path: String) -> POSIXError? {
        var consumed: FSItem.Attribute = []
        if a.isValid(.size) {
            guard truncate(path, off_t(a.size)) == 0 else { return lastError() }
            consumed.insert(.size)
        }
        if a.isValid(.mode) {
            guard chmod(path, mode_t(a.mode & 0o7777)) == 0 else { return lastError() }
            consumed.insert(.mode)
        }
        if a.isValid(.accessTime) || a.isValid(.modifyTime) {
            var times = [timespec(tv_sec: 0, tv_nsec: Int(UTIME_OMIT)), timespec(tv_sec: 0, tv_nsec: Int(UTIME_OMIT))]
            if a.isValid(.accessTime) {
                times[0] = a.accessTime
                consumed.insert(.accessTime)
            }
            if a.isValid(.modifyTime) {
                times[1] = a.modifyTime
                consumed.insert(.modifyTime)
            }
            guard utimensat(AT_FDCWD, path, times, AT_SYMLINK_NOFOLLOW) == 0 else { return lastError() }
        }
        for ignored in [FSItem.Attribute.uid, .gid, .flags, .birthTime, .backupTime, .addedTime] where a.isValid(ignored) {
            consumed.insert(ignored)
        }
        a.consumedAttributes = consumed
        return nil
    }

    // MARK: Path configuration

    var maximumLinkCount: Int { Int(LINK_MAX) }
    var maximumNameLength: Int { 255 }
    var restrictsOwnershipChanges: Bool { true }
    var truncatesLongNames: Bool { false }

    // MARK: Volume

    var supportedVolumeCapabilities: FSVolume.SupportedCapabilities {
        let c = FSVolume.SupportedCapabilities()
        c.supportsHardLinks = false
        c.supportsSymbolicLinks = true
        c.supportsPersistentObjectIDs = true
        c.supports64BitObjectIDs = true
        // The host's APFS (OPEN: a volume on a case-sensitive file system)
        c.caseFormat = .insensitiveCasePreserving
        return c
    }

    var volumeStatistics: FSStatFSResult {
        var s = statfs()
        let r = FSStatFSResult(fileSystemTypeName: "omaflakefs")
        if statfs(rootPath, &s) == 0 {
            r.blockSize = Int(s.f_bsize)
            r.ioSize = Int(s.f_iosize)
            r.totalBlocks = UInt64(s.f_blocks)
            r.availableBlocks = UInt64(s.f_bavail)
            r.freeBlocks = UInt64(s.f_bfree)
            r.totalFiles = UInt64(s.f_files)
            r.freeFiles = UInt64(s.f_ffree)
        }
        return r
    }

    func mount(options: FSTaskOptions, replyHandler: @escaping ((any Error)?) -> Void) { replyHandler(nil) }
    func unmount(replyHandler: @escaping () -> Void) { replyHandler() }
    func synchronize(flags: FSSyncFlags, replyHandler: @escaping ((any Error)?) -> Void) { replyHandler(nil) }

    func activate(options: FSTaskOptions, replyHandler: @escaping (FSItem?, (any Error)?) -> Void) {
        guard let root = item(rootPath) else { return replyHandler(nil, lastError()) }
        replyHandler(root, nil)
    }

    func deactivate(options: FSDeactivateOptions = [], replyHandler: @escaping ((any Error)?) -> Void) {
        replyHandler(nil)
    }

    func getAttributes(_ desiredAttributes: FSItem.GetAttributesRequest, of item: FSItem,
                       replyHandler: @escaping (FSItem.Attributes?, (any Error)?) -> Void) {
        guard let i = item as? FolderItem else { return replyHandler(nil, POSIXError(.EIO)) }
        guard let a = attributes(i.path, id: i.id, parent: parentID(of: i.path)) else {
            return replyHandler(nil, lastError())
        }
        replyHandler(a, nil)
    }

    func setAttributes(_ newAttributes: FSItem.SetAttributesRequest, on item: FSItem,
                       replyHandler: @escaping (FSItem.Attributes?, (any Error)?) -> Void) {
        guard let i = item as? FolderItem else { return replyHandler(nil, POSIXError(.EIO)) }
        if let e = apply(newAttributes, to: i.path) { return replyHandler(nil, e) }
        replyHandler(attributes(i.path, id: i.id, parent: parentID(of: i.path)), nil)
    }

    func lookupItem(named name: FSFileName, inDirectory directory: FSItem,
                    replyHandler: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void) {
        guard let d = directory as? FolderItem, let n = name.string else {
            return replyHandler(nil, nil, POSIXError(.EINVAL))
        }
        guard let i = item((d.path as NSString).appendingPathComponent(n)) else {
            return replyHandler(nil, nil, POSIXError(.ENOENT))
        }
        replyHandler(i, name, nil)
    }

    func reclaimItem(_ item: FSItem, replyHandler: @escaping ((any Error)?) -> Void) {
        if let i = item as? FolderItem, i.id != FSItem.Identifier.rootDirectory.rawValue {
            lock.lock()
            if items[i.id] === i { items[i.id] = nil }
            lock.unlock()
        }
        replyHandler(nil)
    }

    func readSymbolicLink(_ item: FSItem, replyHandler: @escaping (FSFileName?, (any Error)?) -> Void) {
        guard let i = item as? FolderItem,
              let target = try? FileManager.default.destinationOfSymbolicLink(atPath: i.path) else {
            return replyHandler(nil, POSIXError(.EIO))
        }
        replyHandler(FSFileName(string: target), nil)
    }

    func createItem(named name: FSFileName, type: FSItem.ItemType, inDirectory directory: FSItem,
                    attributes newAttributes: FSItem.SetAttributesRequest,
                    replyHandler: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void) {
        guard let d = directory as? FolderItem, let n = name.string else {
            return replyHandler(nil, nil, POSIXError(.EINVAL))
        }
        let p = (d.path as NSString).appendingPathComponent(n)
        let fallback: UInt32 = type == .directory ? 0o755 : 0o644
        let mode = mode_t(newAttributes.isValid(.mode) ? newAttributes.mode & 0o7777 : fallback)
        switch type {
        case .directory:
            guard mkdir(p, mode) == 0 else { return replyHandler(nil, nil, lastError()) }
        case .file:
            let fd = open(p, O_CREAT | O_EXCL | O_WRONLY, mode)
            guard fd >= 0 else { return replyHandler(nil, nil, lastError()) }
            close(fd)
        default:
            return replyHandler(nil, nil, POSIXError(.ENOTSUP))
        }
        unquarantine(p)
        if newAttributes.isValid(.size) || newAttributes.isValid(.modifyTime) || newAttributes.isValid(.accessTime) {
            _ = apply(newAttributes, to: p)
        }
        newAttributes.consumedAttributes.insert(.mode)
        guard let i = item(p) else { return replyHandler(nil, nil, lastError()) }
        replyHandler(i, name, nil)
    }

    func createSymbolicLink(named name: FSFileName, inDirectory directory: FSItem,
                            attributes newAttributes: FSItem.SetAttributesRequest, linkContents contents: FSFileName,
                            replyHandler: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void) {
        guard let d = directory as? FolderItem, let n = name.string, let target = contents.string else {
            return replyHandler(nil, nil, POSIXError(.EINVAL))
        }
        let p = (d.path as NSString).appendingPathComponent(n)
        guard symlink(target, p) == 0 else { return replyHandler(nil, nil, lastError()) }
        unquarantine(p)
        guard let i = item(p) else { return replyHandler(nil, nil, lastError()) }
        replyHandler(i, name, nil)
    }

    /// Hard links: not on this volume (`supportsHardLinks` false); OPEN
    func createLink(to item: FSItem, named name: FSFileName, inDirectory directory: FSItem,
                    replyHandler: @escaping (FSFileName?, (any Error)?) -> Void) {
        replyHandler(nil, POSIXError(.ENOTSUP))
    }

    func removeItem(_ item: FSItem, named name: FSFileName, fromDirectory directory: FSItem,
                    replyHandler: @escaping ((any Error)?) -> Void) {
        guard let i = item as? FolderItem else { return replyHandler(POSIXError(.EIO)) }
        var st = stat()
        guard lstat(i.path, &st) == 0 else { return replyHandler(lastError()) }
        let r = (st.st_mode & S_IFMT) == S_IFDIR ? rmdir(i.path) : unlink(i.path)
        replyHandler(r == 0 ? nil : lastError())
    }

    func renameItem(_ item: FSItem, inDirectory sourceDirectory: FSItem, named sourceName: FSFileName,
                    to destinationName: FSFileName, inDirectory destinationDirectory: FSItem, overItem: FSItem?,
                    replyHandler: @escaping (FSFileName?, (any Error)?) -> Void) {
        guard let i = item as? FolderItem, let to = destinationDirectory as? FolderItem,
              let n = destinationName.string else {
            return replyHandler(nil, POSIXError(.EINVAL))
        }
        let destination = (to.path as NSString).appendingPathComponent(n)
        guard rename(i.path, destination) == 0 else { return replyHandler(nil, lastError()) }
        // The item and everything known below it now live under the new path
        let old = i.path
        lock.lock()
        for known in items.values where known.path == old || known.path.hasPrefix(old + "/") {
            known.path = destination + known.path.dropFirst(old.count)
        }
        lock.unlock()
        replyHandler(destinationName, nil)
    }

    func enumerateDirectory(_ directory: FSItem, startingAt cookie: FSDirectoryCookie,
                            verifier: FSDirectoryVerifier, attributes: FSItem.GetAttributesRequest?,
                            packer: FSDirectoryEntryPacker,
                            replyHandler: @escaping (FSDirectoryVerifier, (any Error)?) -> Void) {
        guard let d = directory as? FolderItem else { return replyHandler(verifier, POSIXError(.EIO)) }
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: d.path) else {
            return replyHandler(verifier, lastError())
        }
        // FSKit adds "." and ".." itself; the cookie is the index in the sorted names
        let entries = names.sorted()
        var index = Int(cookie.rawValue)
        while index < entries.count {
            let p = (d.path as NSString).appendingPathComponent(entries[index])
            var st = stat()
            guard lstat(p, &st) == 0, let i = item(p) else {
                index += 1
                continue
            }
            let attrs = attributes != nil ? self.attributes(p, id: i.id, parent: d.id) : nil
            let packed = packer.packEntry(name: FSFileName(string: entries[index]), itemType: Self.itemType(st.st_mode),
                                          itemID: FSItem.Identifier(rawValue: i.id) ?? .invalid,
                                          nextCookie: FSDirectoryCookie(rawValue: UInt64(index + 1)), attributes: attrs)
            if !packed { break }
            index += 1
        }
        replyHandler(FSDirectoryVerifier(rawValue: 1), nil)
    }

    // MARK: Reading and writing

    func read(from item: FSItem, at offset: off_t, length: Int, into buffer: FSMutableFileDataBuffer,
              replyHandler: @escaping (Int, (any Error)?) -> Void) {
        guard let i = item as? FolderItem else { return replyHandler(0, POSIXError(.EIO)) }
        let fd = open(i.path, O_RDONLY)
        guard fd >= 0 else { return replyHandler(0, lastError()) }
        defer { close(fd) }
        let n = buffer.withUnsafeMutableBytes { pread(fd, $0.baseAddress, min(length, $0.count), offset) }
        n < 0 ? replyHandler(0, lastError()) : replyHandler(n, nil)
    }

    func write(contents: Data, to item: FSItem, at offset: off_t, replyHandler: @escaping (Int, (any Error)?) -> Void) {
        guard let i = item as? FolderItem else { return replyHandler(0, POSIXError(.EIO)) }
        let fd = open(i.path, O_WRONLY)
        guard fd >= 0 else { return replyHandler(0, lastError()) }
        defer { close(fd) }
        let n = contents.withUnsafeBytes { pwrite(fd, $0.baseAddress, $0.count, offset) }
        n < 0 ? replyHandler(0, lastError()) : replyHandler(n, nil)
    }
}
