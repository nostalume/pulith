mod manifest;
mod resolve;

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let manifest_path = args
        .next()
        .expect("usage: versioned_tool <manifest.toml> <layout-root>");
    let root = args
        .next()
        .expect("usage: versioned_tool <manifest.toml> <layout-root>");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = manifest::Manifest::parse(&text).expect("parse manifest");
    let layout = resolve::Layout {
        root: PathBuf::from(root),
    };
    let resolved = resolve::resolve(manifest, &layout).expect("resolve");

    println!(
        "plan: {}@{}",
        resolved.manifest.name, resolved.manifest.version
    );
    println!("  source:   {}", describe_source(&resolved.manifest));
    println!("  target:   {}", resolved.target.path.display());
    match &resolved.manifest.expose {
        Some(expose) => println!("  expose:   {expose}"),
        None => println!("  expose:   (tree root)"),
    }
    match &resolved.view {
        Some(view) => println!("  link_at:  {}", view.display()),
        None => println!("  link_at:  (no view)"),
    }
}

fn describe_source(manifest: &manifest::Manifest) -> String {
    match &manifest.source {
        manifest::Source::Url { url } => url.clone(),
        manifest::Source::Local { path } => path.display().to_string(),
    }
}
