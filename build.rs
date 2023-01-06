const PLAT_TOOLS_URIS: &[(&str, &str)] = &[
    ("https://dl.google.com/android/repository/platform-tools-latest-windows.zip", "./plat_tools_win.zip"),
    ("https://dl.google.com/android/repository/platform-tools-latest-darwin.zip", "./plat_tools_mac.zip"),
    ("https://dl.google.com/android/repository/platform-tools-latest-linux.zip", "./plat_tools_linux.zip")
];

fn main() {
    // Download platform-tools for the appropriate platform, if missing

    for (uri, download_to) in PLAT_TOOLS_URIS {
        println!("Downloading {download_to} from {uri}");

        if let Ok(_) = std::fs::File::open(download_to) {
            continue;
        }

        let mut writer = std::fs::File::create(download_to).expect("Failed to write platform-tools");
        let mut reader = ureq::get(uri)
            .call()
            .expect("Failed to download platform-tools")
            .into_reader();

        std::io::copy(&mut reader, &mut writer).expect("Failed to copy to platform-tools file");
    }
}