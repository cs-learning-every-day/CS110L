use linked_list::LinkedList;

use crate::linked_list::ComputeNorm;
pub mod linked_list;

fn main() {
    let mut list: LinkedList<u32> = LinkedList::new();
    assert!(list.is_empty());
    assert_eq!(list.get_size(), 0);
    for i in 1..12 {
        list.push_front(i);
    }
    println!("{}", list);
    println!("list size: {}", list.get_size());
    println!("top element: {}", list.pop_front().unwrap());
    println!("{}", list);
    println!("size: {}", list.get_size());
    println!("{}", list.to_string()); // ToString impl for anything impl Display

    let mut list_clone = list.clone();
    list_clone.push_front(1);
    println!("clone list and push front 1: {}", list_clone);
    println!("origin list: {}", list);

    let mut l2: LinkedList<String> = LinkedList::new();
    let hl = String::from("hello");
    l2.push_front(hl);
    println!("{}", l2);

    let mut l3 = l2.clone();
    println!("test equal: {}", l3 == l2);

    let mut tmp = l3.pop_front().unwrap();
    tmp.push_str("string");
    println!("test equal: {}", l3 == l2);
    println!(
        "deep clone: old: {},new:{},l3:{}",
        l2.pop_front().unwrap(),
        tmp,
        l3
    );

    let mut flst: LinkedList<f64> = LinkedList::new();
    flst.push_front(3.0);
    flst.push_front(4.0);
    // If you implement iterator trait:
    for val in &flst {
        println!("{}", val);
    }

    println!("{}", flst.compute_norm());
}
