use serde::Deserialize;

#[derive(Deserialize)]
pub struct NotifyMachineRequest {
    pub machine_id: String,
    pub user_id: String,
    pub message: String, // Added message field for notification content
}
