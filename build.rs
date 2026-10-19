fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icons/neptune-finder.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/neptune-finder.ico")
            .set("ProductName", "Neptune Finder")
            .set("FileDescription", "Neptune Finder file explorer")
            .compile()?;
    }
    Ok(())
}
