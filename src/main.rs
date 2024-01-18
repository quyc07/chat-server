use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::{Form, Json, Router};
use axum::routing::get;
use serde::Deserialize;

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        .route("/", get(index))
        .route("/query", get(query))
        .route("/form", get(show_form).post(get_form));
    let app = app.fallback(handler_404);
    // run it
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}

async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "该页面找不到了")
}

#[derive(Debug, Deserialize)]
struct Params {
    name: Option<String>,
    email: Option<String>,
}

async fn query(Query(params): Query<Params>) -> String {
    format!("{params:?}")
}

async fn show_form() -> Html<&'static str> {
    Html(
        r#"
        <!doctype html>
        <html>
            <head></head>
            <body>
                <form action="/form" method="post">
                    <label for="name">
                        Enter your name:
                        <input type="text" name="name">
                    </label>

                    <label>
                        Enter your email:
                        <input type="text" name="email">
                    </label>

                    <input type="submit" value="Subscribe!">
                </form>
            </body>
        </html>
        "#,
    )
}

async fn get_form(Form(params): Form<Params>) {
    dbg!(params);
}
