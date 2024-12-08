use serde::Serialize;

#[derive(Serialize)]
pub struct ApiResponse<T, E> {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data:Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error:Option<E>
}

impl<T, E> ApiResponse<T, E> {

    pub(crate) fn success(message: &str, data: T) -> Self {

        ApiResponse {
            success: true,
            message: message.to_string(),
            data: Some(data),
            error: None
        }

    }

    pub(crate) fn error(message: &str, error: E) -> Self {
        ApiResponse {

            success: false,
            message: message.to_string(),
            data: None,
            error: Some(error)
        }
    }

}
