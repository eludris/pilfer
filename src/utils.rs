#[macro_export]
macro_rules! prompt {
    ($($arg:tt)*) => {{
        print!($($arg)*);
        stdout().flush().unwrap();
        let mut input = String::new();
        stdin().read_line(&mut input).unwrap();
        input.trim().to_string()
    }};
}
