fn test3() {
    let s1 = String::from("hello");
    let mut v = Vec::new();
    v.push(s1);
    let s2: &String = &v[0];
    println!("{}", s2);
}

fn drip_drop() -> String {
    let s = String::from("hello world!");
    return s;
}

fn test1() {
    let mut s = String::from("hello");
    let ref1 = &s;
    let ref2 = &ref1;
    let ref3 = &ref2;
    println!("{}", ref3.to_uppercase());
    s = String::from("goodbye");
}

fn main() {
    test1();
    let s = drip_drop();
    println!("{}", s.to_lowercase());
    test3();
}
