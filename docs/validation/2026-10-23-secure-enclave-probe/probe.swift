// Cranibox Secure Enclave probe: can an ad-hoc signed binary create a Secure Enclave
// key without the keychain, persist it as a file, and sign only after Touch ID?
import CryptoKit
import Foundation
import LocalAuthentication

func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }
func fail(_ message: String) -> Never {
    print("{\"ok\":false,\"error\":\"\(message)\"}")
    exit(1)
}

let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("cranibox-se-probe")
try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
let keyFile = dir.appendingPathComponent("admin.se-key")

guard SecureEnclave.isAvailable else { fail("Secure Enclave is not available") }

let context = LAContext()
context.localizedReason = "Cranibox 原型：签名测试消息"
let key: SecureEnclave.P256.Signing.PrivateKey
var created = false
do {
    if let data = try? Data(contentsOf: keyFile) {
        key = try SecureEnclave.P256.Signing.PrivateKey(
            dataRepresentation: data, authenticationContext: context)
    } else {
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], &error)
        else { fail("access control: \(String(describing: error))") }
        key = try SecureEnclave.P256.Signing.PrivateKey(
            accessControl: access, authenticationContext: context)
        try key.dataRepresentation.write(to: keyFile, options: .atomic)
        created = true
    }
} catch {
    fail("key: \(error)")
}

let message = Data("cranibox-se-probe \(Date().timeIntervalSince1970)".utf8)
do {
    // Touch ID is required here, by the Secure Enclave, not by this process.
    let signature = try key.signature(for: message)
    print("""
    {"ok":true,"created":\(created),"key_file":"\(keyFile.path)",\
    "public_x963":"\(hex(key.publicKey.x963Representation))",\
    "message":"\(hex(message))","signature_der":"\(hex(signature.derRepresentation))",\
    "self_verified":\(key.publicKey.isValidSignature(signature, for: message))}
    """)
} catch {
    fail("sign: \(error)")
}
