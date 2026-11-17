//! JSON request bodies with errors in the API's own format.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use opsbox_proto::ErrorCode;
use serde::de::DeserializeOwned;

use crate::HttpError;

/// Like `axum::Json`, but a malformed body is a `validation` error in the spec format
/// instead of plain text.
pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = HttpError;
    async fn from_request(req: Request, state: &S) -> Result<Self, HttpError> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(Self(v)),
            Err(JsonRejection::BytesRejection(_)) => Err(HttpError::new(
                ErrorCode::Validation,
                "The request body is too large or could not be read.",
            )),
            Err(_) => Err(HttpError::new(
                ErrorCode::Validation,
                "The request body is not valid JSON for this endpoint.",
            )),
        }
    }
}

/// With `Option<ApiJson<T>>` a request without a `Content-Type` has no body (for example a
/// `POST` that only means "do it"), instead of being an error.
impl<S: Send + Sync, T: DeserializeOwned> axum::extract::OptionalFromRequest<S> for ApiJson<T> {
    type Rejection = HttpError;
    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, HttpError> {
        if !req.headers().contains_key(axum::http::header::CONTENT_TYPE) {
            return Ok(None);
        }
        <Self as FromRequest<S>>::from_request(req, state)
            .await
            .map(Some)
    }
}
