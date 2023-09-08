use crossbeam_channel::{self, bounded, Receiver, Sender};
use std::{thread, time};

fn parallel_map<T, U, F>(mut input_vec: Vec<T>, num_threads: usize, f: F) -> Vec<U>
where
    F: FnOnce(T) -> U + Send + Copy + 'static,
    T: Send + 'static,
    U: Send + 'static + Default,
{
    let mut output_vec: Vec<U> = Vec::with_capacity(input_vec.len());
    // TODO: implement parallel map!
    let (input_sender, input_receiver): (Sender<T>, Receiver<T>) = bounded(input_vec.len());
    let (output_sender, output_receiver): (Sender<U>, Receiver<U>) = bounded(input_vec.len());
    for _ in 0..num_threads {
        let input_receiver = input_receiver.clone();
        let output_sender = output_sender.clone();
        thread::spawn(move || {
            while let Ok(input) = input_receiver.recv() {
                let output = f(input);
                output_sender.send(output).unwrap();
            }
        });
    }
    for input in input_vec.drain(..) {
        input_sender.send(input).unwrap();
    }
    drop(input_sender);

    while let Ok(output) = output_receiver.recv() {
        output_vec.push(output);
        if output_vec.len() == output_vec.capacity() {
            break;
        }
    }
    output_vec
}

fn main() {
    let v = vec![6, 7, 8, 9, 10, 1, 2, 3, 4, 5, 12, 18, 11, 5, 20];
    let squares = parallel_map(v, 10, |num| {
        println!("{} squared is {}", num, num * num);
        thread::sleep(time::Duration::from_millis(500));
        num * num
    });
    println!("squares: {:?}", squares);
}
