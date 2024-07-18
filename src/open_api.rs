use crate::group::GroupApi;
use crate::user::UserApi;
use utoipa::OpenApi;
use utoipa_swagger_ui::{SwaggerUi, Url};

pub async fn swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/swagger-ui").urls(vec![
        (
            Url::new("User Api", "/api-docs/user.json"),
            UserApi::openapi(),
        ),
        (
            Url::new("Group Api", "/api-docs/group.json"),
            GroupApi::openapi(),
        ),
    ])
}
