mod manifest;
mod realize;

use std::path::Path;

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
        .expect("usage: vtool install <manifest.toml> <layout-root>");
    let root = args
        .next()
        .expect("usage: vtool install <manifest.toml> <layout-root>");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = manifest::Manifest::parse(&text).expect("parse manifest");
    let resolved = manifest.resolve(Path::new(&root)).expect("resolve");
    let outcome = resolved.install(Path::new(&root)).expect("install");
    println!("outcome: {outcome:?}");
}

fn plan(args: &mut impl Iterator<Item = String>) {
    let manifest_path = args
        .next()
        .expect("usage: vtool plan <manifest.toml> <layout-root>");
    let root = args
        .next()
        .expect("usage: vtool plan <manifest.toml> <layout-root>");
    let text = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest = manifest::Manifest::parse(&text).expect("parse manifest");
    let resolved = manifest.resolve(Path::new(&root)).expect("resolve");

    println!(
        "plan: {}@{}\n",
        resolved.manifest.name.as_str(),
        resolved.manifest.version.as_str()
    );
    println!("  source:   {}", describe_source(&resolved.manifest));
    println!("  target:   {}", resolved.target.display());
    match &resolved.manifest.expose {
        Some(expose) => println!("  expose:   {}", expose.display()),
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
        manifest::Source::Url { url } => url.as_str().to_string(),
        manifest::Source::Local { path } => path.display().to_string(),
    }) {
        Some(described) => described,
        None => "(no source for this platform)".to_string(),
    }
}
