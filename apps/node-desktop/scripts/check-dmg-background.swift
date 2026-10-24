import AppKit
import Foundation

// Validate with the same image framework used by Finder, including the copy
// mounted from the final DMG in CI. Pixel dimensions alone miss this regression.
struct Config: Decodable {
    struct Bundle: Decodable {
        struct MacOS: Decodable {
            struct Dmg: Decodable {
                struct Size: Decodable { let width: Int; let height: Int }
                let windowSize: Size
            }
            let dmg: Dmg
        }
        let macOS: MacOS
    }
    let bundle: Bundle
}

precondition(CommandLine.arguments.count == 3, "Usage: check-dmg-background.swift IMAGE CONFIG")
let config = try JSONDecoder().decode(Config.self,
    from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2])))
let window = config.bundle.macOS.dmg.windowSize
let expected = NSSize(width: CGFloat(window.width), height: CGFloat(window.height))
guard let image = NSImage(contentsOfFile: CommandLine.arguments[1]) else {
    fatalError("Cannot load DMG background")
}
precondition(image.size == expected, "Background logical size must match the DMG window")
for scale in [1, 2] {
    precondition(image.representations.contains {
        $0.pixelsWide == window.width * scale && $0.pixelsHigh == window.height * scale
            && $0.size == expected
    }, "Missing correctly sized \(scale)x background representation")
}
print("DMG background verified: \(window.width)×\(window.height) points, 1x and 2x")
