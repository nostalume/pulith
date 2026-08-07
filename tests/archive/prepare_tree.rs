use std::path::Path;

fn archive_root(tree: &pulith::archive::PreparedTree) -> &Path {
    tree.root()
}

#[test]
fn prepared_tree_exposes_root_by_shared_reference() {
    let _ = archive_root as fn(&pulith::archive::PreparedTree) -> &Path;
}
