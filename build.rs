use windres::Build;

fn main() {
    Build::new().compile("ttlitex.rc").unwrap();
}
