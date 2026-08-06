use std::path::Path;

fn archive_root<A>(tree: &pulith::archive::ArchiveTree<A>) -> &Path {
    tree.root()
}

#[test]
fn archive_tree_exposes_root_by_shared_reference() {
    let _ = archive_root::<()> as fn(&pulith::archive::ArchiveTree<()>) -> &Path;
}
