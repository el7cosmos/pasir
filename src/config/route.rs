use anyhow::Context;
use hyper::body::Incoming;
use hyper::http::{HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Routes {
  routes: Vec<Route>,
}

impl Routes {
  pub(crate) fn from_file(path: &str) -> anyhow::Result<Self> {
    let content = std::fs::read_to_string(path)
      .with_context(|| format!("Failed to read config file: {path}"))?;

    let routes =
      toml::from_str(&content).with_context(|| format!("Failed to parse config file: {path}"))?;

    Ok(routes)
  }

  pub(crate) fn served_route(&self, request: &Request<Incoming>) -> Option<Route> {
    self
      .routes
      .iter()
      .find(|route| route.serve.is_some() && route.matches_request(request))
      .cloned()
  }
}

impl ApplyActions for Routes {
  fn apply_actions<B>(&self, response: &mut Response<B>) {
    for route in &self.routes {
      if route.matches_response(response) {
        route.apply_actions(response);
      }
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Route {
  #[serde(rename = "match")]
  route_match: RouteMatch,
  #[serde(default)]
  action: Option<RouteAction>,
  #[serde(default)]
  serve: Option<RouteServe>,
}

impl Route {
  pub(crate) fn serve(&mut self) -> RouteServe {
    self.serve.take().unwrap()
  }
}

impl MatchesRequest for Route {
  fn matches_request<B>(&self, request: &Request<B>) -> bool {
    self.route_match.matches_request(request)
  }
}

impl MatchesResponse for Route {
  fn matches_response<B>(&self, response: &Response<B>) -> bool {
    self.serve.is_none() && self.route_match.matches_response(response)
  }
}

impl ApplyActions for Route {
  fn apply_actions<B>(&self, response: &mut Response<B>) {
    if let Some(action) = &self.action {
      if let Some(status) = action.status {
        *response.status_mut() = status
      }
      action.response_headers.apply_actions(response)
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RouteMatch {
  #[serde(default, deserialize_with = "deserialize_uri")]
  uri: Option<Regex>,
  #[serde(default, deserialize_with = "deserialize_headers")]
  response_headers: HashMap<HeaderName, Regex>,
}

impl MatchesRequest for RouteMatch {
  fn matches_request<B>(&self, request: &Request<B>) -> bool {
    match &self.uri {
      None => true,
      Some(regex) => regex.is_match(request.uri().path()),
    }
  }
}

impl MatchesResponse for RouteMatch {
  fn matches_response<B>(&self, response: &Response<B>) -> bool {
    for (key, value) in self.response_headers.iter() {
      if !response.headers().contains_key(key) {
        return false;
      }
      if !value.is_match(response.headers().get(key).unwrap().to_str().unwrap()) {
        return false;
      }
    }
    true
  }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RouteAction {
  #[serde(default, deserialize_with = "deserialize_status")]
  status: Option<StatusCode>,
  #[serde(default)]
  response_headers: ResponseHeaderAction,
}

type ResponseHeaderActionOption = Option<HashMap<HeaderName, HeaderValue>>;

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ResponseHeaderAction {
  #[serde(default, deserialize_with = "deserialize_action_headers")]
  insert: ResponseHeaderActionOption,
  #[serde(default, deserialize_with = "deserialize_action_headers")]
  append: ResponseHeaderActionOption,
  remove: Option<Vec<String>>,
}

impl ApplyActions for ResponseHeaderAction {
  fn apply_actions<B>(&self, response: &mut Response<B>) {
    if let Some(insert) = &self.insert {
      for (key, value) in insert {
        response.headers_mut().insert(key, value.clone());
      }
    }
    if let Some(append) = &self.append {
      for (key, value) in append {
        response.headers_mut().append(key, value.clone());
      }
    }
    if let Some(remove) = &self.remove {
      for key in remove {
        response.headers_mut().remove(key);
      }
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RouteServe {
  Php,
  Default,
  Static,
}

fn deserialize_uri<'de, D>(deserializer: D) -> Result<Option<Regex>, D::Error>
where
  D: Deserializer<'de>,
{
  RegexBuilder::new(&String::deserialize(deserializer)?)
    .case_insensitive(true)
    .build()
    .map(Some)
    .map_err(serde::de::Error::custom)
}

fn deserialize_status<'de, D>(deserializer: D) -> Result<Option<StatusCode>, D::Error>
where
  D: Deserializer<'de>,
{
  let status = u16::deserialize(deserializer)?;
  StatusCode::from_u16(status).map(|t| t.into()).map_err(serde::de::Error::custom)
}

fn deserialize_headers<'de, D>(deserializer: D) -> Result<HashMap<HeaderName, Regex>, D::Error>
where
  D: Deserializer<'de>,
{
  let map = Vec::<HashMap<String, String>>::deserialize(deserializer)?;
  let hash_map = map.into_iter().flat_map(|m| m.into_iter()).collect::<HashMap<String, String>>();
  hash_map
    .into_iter()
    .map(|(key, value)| {
      Ok((key.parse()?, RegexBuilder::new(&value).case_insensitive(true).build()?))
    })
    .collect::<anyhow::Result<HashMap<HeaderName, Regex>>>()
    .map_err(serde::de::Error::custom)
}

fn deserialize_action_headers<'de, D>(
  deserializer: D,
) -> Result<ResponseHeaderActionOption, D::Error>
where
  D: Deserializer<'de>,
{
  let vec = Vec::<HashMap<String, String>>::deserialize(deserializer)?;
  let hash_map = vec.into_iter().flat_map(|m| m.into_iter()).collect::<HashMap<String, String>>();
  hash_map
    .into_iter()
    .map(|(key, value)| Ok(Some((key.parse()?, value.parse()?))))
    .collect::<anyhow::Result<ResponseHeaderActionOption>>()
    .map_err(serde::de::Error::custom)
}

trait MatchesRequest {
  fn matches_request<B>(&self, request: &Request<B>) -> bool;
}

trait MatchesResponse {
  fn matches_response<B>(&self, response: &Response<B>) -> bool;
}

pub(crate) trait ApplyActions {
  fn apply_actions<B>(&self, response: &mut Response<B>);
}

#[cfg(test)]
mod tests {
  use crate::config::route::Routes;

  #[test]
  fn test_deserialize_route() {
    let routes = Routes::from_file("tests/fixtures/routes.toml");
    assert_eq!(routes.is_ok(), true);
  }
}
