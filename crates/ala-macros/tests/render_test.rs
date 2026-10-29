#[test]
fn test_derive_render() {
    use ala_macros::Render;

    #[derive(Render)]
    struct _Element;
}
