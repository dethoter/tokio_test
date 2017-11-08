use std::time::Duration;

#[derive(Serialize, Deserialize, Debug)]
pub enum CountResponse {
    Duration(u64, Duration),
    Timeout(u64),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CountRequest(pub u64);
