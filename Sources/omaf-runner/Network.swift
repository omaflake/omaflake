// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Virtualization
import OmaflakeKit

extension NetworkCard {
    /// The second card on one of omaflake's networks (spec §11.12), on the socket `open()` gave
    func device(_ handle: FileHandle) throws -> VZVirtioNetworkDeviceConfiguration {
        let net = VZVirtioNetworkDeviceConfiguration()
        net.attachment = VZFileHandleNetworkDeviceAttachment(fileHandle: handle)
        guard let address = VZMACAddress(string: mac) else { throw VMError("Invalid MAC address for a network: \(mac)") }
        net.macAddress = address
        return net
    }
}
