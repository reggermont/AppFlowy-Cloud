use actix_http::header::HeaderName;
use std::future::{ready, Ready};
use tracing::{Instrument, Level};

use actix_service::{forward_ready, Service, Transform};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use futures_util::future::LocalBoxFuture;
use reqwest::header::HeaderValue;

const X_REQUEST_ID: &str = "x-request-id";
pub struct RequestIdMiddleware;

impl<S, B> Transform<S, ServiceRequest> for RequestIdMiddleware
where
  S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
  S::Future: 'static,
  B: 'static,
{
  type Response = ServiceResponse<B>;
  type Error = actix_web::Error;
  type Transform = RequestIdMiddlewareService<S>;
  type InitError = ();
  type Future = Ready<Result<Self::Transform, Self::InitError>>;

  fn new_transform(&self, service: S) -> Self::Future {
    ready(Ok(RequestIdMiddlewareService { service }))
  }
}

pub struct RequestIdMiddlewareService<S> {
  service: S,
}

impl<S, B> Service<ServiceRequest> for RequestIdMiddlewareService<S>
where
  S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
  S::Future: 'static,
  B: 'static,
{
  type Response = ServiceResponse<B>;
  type Error = actix_web::Error;
  type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

  forward_ready!(service);

  fn call(&self, mut req: ServiceRequest) -> Self::Future {
    // Skip generate request id for metrics requests
    if req.path() == "/metrics" {
      let fut = self.service.call(req);
      Box::pin(fut)
    } else {
      let request_id = match get_request_id(&req) {
        Some(request_id) => request_id,
        None => {
          let request_id = uuid::Uuid::new_v4().to_string();
          if let Ok(header_value) = HeaderValue::from_str(&request_id) {
            req
              .headers_mut()
              .insert(HeaderName::from_static(X_REQUEST_ID), header_value);
          }
          request_id
        },
      };

      let span = tracing::span!(Level::INFO, "request_id", request_id = %request_id);
      let fut = self.service.call(req);

      Box::pin(async move {
        let mut res = fut.instrument(span).await?;

        // Insert the request id to the response header
        if let Ok(header_value) = HeaderValue::from_str(&request_id) {
          res
            .headers_mut()
            .insert(HeaderName::from_static(X_REQUEST_ID), header_value);
        }
        Ok(res)
      })
    }
  }
}

pub fn get_request_id(req: &ServiceRequest) -> Option<String> {
  match req.headers().get(HeaderName::from_static(X_REQUEST_ID)) {
    Some(h) => match h.to_str() {
      Ok(s) => Some(s.to_owned()),
      Err(e) => {
        tracing::error!("Failed to get request id from header: {}", e);
        None
      },
    },
    None => None,
  }
}
