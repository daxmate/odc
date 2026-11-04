// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ODictKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        // UI-free binding over the ODC FFI layer; safe to embed on its own.
        .library(name: "ODictCore", targets: ["ODictCore"]),
        .library(name: "ODictKit", targets: ["ODictKit"]),
    ],
    dependencies: [
        // Reads .xlsx workbooks for the editor's CSV/Excel import.
        .package(url: "https://github.com/CoreOffice/CoreXLSX.git", exact: "0.14.2"),
    ],
    targets: [
        // Built by scripts/build-odc-xcframework.sh; gitignored.
        .binaryTarget(name: "ODCFFI", path: "Frameworks/ODCFFI.xcframework"),
        .target(name: "ODictCore", dependencies: ["ODCFFI"]),
        .target(
            name: "ODictKit",
            dependencies: [
                "ODictCore",
                .product(name: "CoreXLSX", package: "CoreXLSX"),
            ]
        ),
        .testTarget(
            name: "ODictKitTests",
            dependencies: ["ODictKit"],
            // Read from disk via #filePath; not bundled into the test target.
            exclude: ["Fixtures"]
        ),
        .testTarget(
            name: "ODictCoreTests",
            dependencies: ["ODictCore"]
        ),
    ]
)
