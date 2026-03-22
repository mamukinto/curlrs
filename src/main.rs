use std::{env::args, time::Instant};

#[tokio::main]
async fn main() {
    let now = Instant::now();

    let url = args().nth(1).unwrap_or("https://dogapi.dog/api/v2/breeds".to_string());

    let client = reqwest::Client::new();

    let response = client.get(url)
        .send().await.unwrap();

    let elapsed = now.elapsed();

    println!("status: {} in {:.2}ms", response.status(), elapsed.as_millis());
}
