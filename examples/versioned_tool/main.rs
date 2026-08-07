mod manifest;
mod realize;
mod resolve;

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "plan".to_string());
    match command.as_str() {
        "plan" => plan(&mut args),
        "install" => install(&mut args),
        other => panic!("unknown command {other:?}: expected `plan` or `install`"),
    }
}

fn install(args: &mut impl Iterator<Item = String>) {
    let manifest_path = args
        .next()
        .expect("usage: versioned_tool install <manifest.toml> <layout-root>");
    let root = args
        .next()
        .expect("usage: versioned_tool install <manifest.toml> <layout-root>");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = manifest::Manifest::parse(&text).expect("parse manifest");
    let layout = resolve::Layout {
        root: PathBuf::from(root),
    };
    let resolved = resolve::resolve(manifest, &layout).expect("resolve");
    let report = realize::install(resolved, &layout).expect("install");
    println!("installed {}", report.target.display());
    match &report.view {
        Some(view) => println!("view {} ({:?})", view.display(), report.outcome),
        None => println!("view: (none declared)"),
    }
}

fn plan(args: &mut impl Iterator<Item = String>) {
    let manifest_path = args
        .next()
        .expect("usage: versioned_tool plan <manifest.toml> <layout-root>");
    let root = args
        .next()
        .expect("usage: versioned_tool plan <manifest.toml> <layout-root>");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = manifest::Manifest::parse(&text).expect("parse manifest");
    let layout = resolve::Layout {
        root: PathBuf::from(root),
    };
    let resolved = resolve::resolve(manifest, &layout).expect("resolve");

    println!(
        "plan: {}@{}\n",
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
    let spec = if cfg!(windows) {
        manifest.windows.as_ref()
    } else {
        manifest.linux.as_ref()
    };
    match spec.map(|spec| match &spec.source {
        manifest::Source::Url { url } => url.clone(),
        manifest::Source::Local { path } => path.display().to_string(),
    }) {
        Some(described) => described,
        None => "(no source for this platform)".to_string(),
    }
}
