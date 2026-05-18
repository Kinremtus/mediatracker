use askama::Template;
use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderValue},
    response::{Html, IntoResponse, Redirect, Response},
};

use crate::app_state::AppState;
use crate::middleware::CurrentUser;

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    username: String,
}

pub async fn get_home(user: CurrentUser) -> Html<String> {
    HomeTemplate {
        username: user.username,
    }
    .render()
    .unwrap()
    .into()
}

pub async fn post_logout(
    State(_state): State<AppState>,
    _user: CurrentUser,
) -> Response {
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static("session_id=; Path=/; HttpOnly; Max-Age=0"),
    );
    response
}
