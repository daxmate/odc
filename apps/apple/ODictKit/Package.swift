// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ODictKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "ODictKit", targets: ["ODictKit"]),
    ],
    targets: [
        // Built by scripts/build-odc-xcframework.sh; gitignored.
        .binaryTarget(name: "ODCFFI", path: "Frameworks/ODCFFI.xcframework"),
        .target(name: "ODictKit", dependencies: ["ODCFFI"]),
        .testTarget(
            name: "ODictKitTests",
            dependencies: ["ODictKit"],
            // Read from disk via #filePath; not bundled into the test target.
            exclude: ["Fixtures"]
        ),
    ]
)
