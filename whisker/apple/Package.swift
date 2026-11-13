// swift-tools-version:5.9
//
// The map's view on iOS and macOS. Whisker's build adds it to the app's
// project, and its codegen plugin registers `MapLibreModule`'s view. It has
// a folder of its own since SwiftPM names a local package after its folder,
// and the crate's, `whisker`, is the name of Whisker's own package.

import PackageDescription

let package = Package(
    name: "maplibre-whisker",
    platforms: [.iOS(.v15), .macOS(.v14)],
    products: [
        .library(name: "MaplibreWhisker", targets: ["MaplibreWhisker"]),
    ],
    dependencies: [
        .package(url: "https://github.com/whiskerrs/whisker.git", exact: "0.1.22"),
    ],
    targets: [
        .target(
            name: "MaplibreWhisker",
            dependencies: [
                .product(name: "WhiskerModule", package: "whisker"),
            ],
            path: "Sources/MaplibreWhisker",
            plugins: [
                .plugin(name: "WhiskerModuleCodegenPlugin", package: "whisker"),
            ]
        ),
    ]
)
