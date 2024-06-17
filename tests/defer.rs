use dark_std::defer;

#[test]
pub fn test_defer() {
    let a = 0;
    defer!(|| {
        println!("{}", a);
    });
}
