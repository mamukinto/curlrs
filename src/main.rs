use std::{env::args, time::Instant};

#[tokio::main]
async fn main() {
    let now = Instant::now();

    let url = args().nth(1);

    let res = reqwest::get(url.or("https://dogapi.dog/api/v2/breeds")).await.unwrap();
    
    let elapsed = now.elapsed();

    println!("status: {} in {:.2}ms", res.status(), elapsed.as_millis());
}
