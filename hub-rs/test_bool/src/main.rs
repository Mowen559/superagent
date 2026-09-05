fn main() {
    let b0 = windows_core::BOOL(0);
    let b1 = windows_core::BOOL(1);
    let b_1 = windows_core::BOOL(-1);
    
    let bool0: bool = b0.into();
    let bool1: bool = b1.into();
    let bool_1: bool = b_1.into();
    
    println!("0 -> {}", bool0);
    println!("1 -> {}", bool1);
    println!("-1 -> {}", bool_1);
}
