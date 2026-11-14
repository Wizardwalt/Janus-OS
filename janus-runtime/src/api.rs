//! HTTP/WebSocket API server.

use crate::{
    executor::{PluginExecutor, PluginSummary},
    hardware::{HardwareAdapterStatus, HardwareManager},
    state::StateManager,
};
use anyhow::Result;
use axum::{
    extract::{Path, State as AxumState},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use janus_core::{
    AuditEntry, CertificationStatus, Config, EngagementScope, LicensedFeature, SignedLicense,
    StateKey, UserRole,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;

#[derive(Clone)]
pub struct ApiState {
    config: Config,
    state_manager: Arc<StateManager>,
    executor: Arc<PluginExecutor>,
    hardware: Arc<HardwareManager>,
}

#[derive(Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    version: String,
    plugins: usize,
}

#[derive(Deserialize)]
struct BootstrapRequest {
    organization_name: String,
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct UserCreateRequest {
    email: String,
    password: String,
    role: UserRole,
}

#[derive(Serialize)]
struct UserResponse {
    user_id: String,
    organization_id: String,
    email: String,
    role: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct ModuleCertificationRequest {
    module_id: String,
    module_sha256: String,
    status: CertificationStatus,
    required_feature: LicensedFeature,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Serialize)]
struct ModuleCertificationResponse {
    module_id: String,
    status: String,
    module_sha256: String,
}

#[derive(Deserialize)]
struct EngagementCreateRequest {
    authorization_reference: String,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
    approved_assets: Vec<String>,
    #[serde(default)]
    approved_evidence_paths: Vec<String>,
    approved_features: Vec<LicensedFeature>,
    #[serde(default = "default_active")]
    active: bool,
}

#[derive(Serialize)]
struct EngagementResponse {
    engagement_id: String,
    organization_id: String,
    active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Deserialize)]
struct LicenseImportRequest {
    license: SignedLicense,
}

#[derive(Serialize)]
struct CurrentUserResponse {
    user_id: String,
    organization_id: String,
    email: String,
    role: String,
}

#[derive(Serialize)]
struct LoginResponse {
    session_token: String,
    expires_at: String,
    user_id: String,
    organization_id: String,
    role: String,
}

#[derive(Serialize)]
struct BootstrapResponse {
    organization_id: String,
    user_id: String,
    email: String,
    role: String,
}

#[derive(Serialize, Deserialize)]
struct ExecuteRequest {
    /// Certified module identifier, normally the plugin ID.
    plugin: String,
    /// Active customer engagement authorizing this execution.
    engagement_id: String,
    /// Exact approved customer asset or evidence location.
    target_asset: String,
    args: serde_json::Value,
}

#[derive(Deserialize)]
struct OwnerDeviceRegisterRequest { device_id: String, device_type: String, password: String }

#[derive(Deserialize)]
struct OwnerDevicePolicyRequest { password: String, policies: serde_json::Value }

#[derive(Serialize, Deserialize)]
struct StateRequest {
    namespace: String,
    key: String,
    value: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct ApiResponse<T> {
    status: String,
    data: Option<T>,
    error: Option<String>,
}

pub struct ApiServer {
    config: Config,
    state_manager: Arc<StateManager>,
    executor: Arc<PluginExecutor>,
    hardware: Arc<HardwareManager>,
}

impl ApiServer {
    pub fn new(
        config: Config,
        state_manager: StateManager,
        executor: PluginExecutor,
        hardware: Arc<HardwareManager>,
    ) -> Self {
        Self {
            config,
            state_manager: Arc::new(state_manager),
            executor: Arc::new(executor),
            hardware,
        }
    }

    pub async fn start(self) -> Result<()> {
        let api_state = ApiState {
            config: self.config.clone(),
            state_manager: self.state_manager,
            executor: self.executor,
            hardware: self.hardware,
        };

        let allowed_origins = self
            .config
            .allowed_origins
            .iter()
            .filter_map(|origin| origin.parse::<HeaderValue>().ok())
            .collect::<Vec<_>>();
        if allowed_origins.is_empty() {
            return Err(anyhow::anyhow!(
                "no valid allowed runtime API origins configured"
            ));
        }

        let app = Router::new()
            .route("/health", get(health))
            .route("/hardware/status", get(hardware_status))
            .route("/auth/bootstrap", post(bootstrap))
            .route("/auth/login", post(login))
            .route("/auth/me", get(current_user))
            .route("/auth/logout", post(logout))
            .route("/auth/users", get(list_users).post(create_user))
            .route("/auth/users/:user_id/disable", post(disable_user))
            .route("/auth/users/:user_id/enable", post(enable_user))
            .route("/owner/devices", get(list_owned_devices).post(register_owned_device))
            .route("/owner/devices/:device_id/policies", post(update_owned_device_policy))
            .route(
                "/auth/users/:user_id/password-reset",
                post(reset_user_password),
            )
            .route("/licenses/import", post(import_license))
            .route("/licenses/status", get(license_status))
            .route(
                "/engagements",
                get(list_engagements).post(create_engagement),
            )
            .route(
                "/engagements/:engagement_id/disable",
                post(disable_engagement),
            )
            .route(
                "/engagements/:engagement_id/enable",
                post(enable_engagement),
            )
            .route("/modules/certifications", post(certify_module))
            .route("/execute", post(execute))
            .route("/plugins", get(list_plugins))
            .route("/state/:namespace/:key", get(get_state).post(set_state))
            .route("/audit/logs", get(get_audit_logs))
            .layer(
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(allowed_origins))
                    .allow_methods([Method::GET, Method::POST])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
            )
            .with_state(api_state);

        let bind_addr = format!("{}:{}", self.config.web_bind, self.config.web_port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        info!("API server listening on {}", bind_addr);

        axum::serve(listener, app).await?;
        Ok(())
    }
}

async fn list_owned_devices(AxumState(state): AxumState<ApiState>, headers: HeaderMap)
    -> (StatusCode, Json<ApiResponse<Vec<serde_json::Value>>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account, Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.owned_devices(&account).await {
        Ok(devices) => (StatusCode::OK, Json(ApiResponse { status: "success".into(), data: Some(devices), error: None })),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse { status: "error".into(), data: None, error: Some(error.to_string()) })),
    }
}

async fn register_owned_device(AxumState(state): AxumState<ApiState>, headers: HeaderMap, Json(request): Json<OwnerDeviceRegisterRequest>)
    -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account, Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.register_owned_device(&account, &request.password, &request.device_id, &request.device_type).await {
        Ok(device) => (StatusCode::CREATED, Json(ApiResponse { status: "success".into(), data: Some(device), error: None })),
        Err(error) => (StatusCode::FORBIDDEN, Json(ApiResponse { status: "error".into(), data: None, error: Some(error.to_string()) })),
    }
}

async fn update_owned_device_policy(AxumState(state): AxumState<ApiState>, headers: HeaderMap, Path(device_id): Path<String>, Json(request): Json<OwnerDevicePolicyRequest>)
    -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account, Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.update_owned_device_policy(&account, &device_id, &request.password, request.policies).await {
        Ok(policies) => (StatusCode::OK, Json(ApiResponse { status: "success".into(), data: Some(policies), error: None })),
        Err(error) => (StatusCode::FORBIDDEN, Json(ApiResponse { status: "error".into(), data: None, error: Some(error.to_string()) })),
    }
}

async fn hardware_status(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<HardwareAdapterStatus>>>) {
    if let Err(error) = authenticated_account(&state.state_manager, &headers).await {
        return unauthorized_response(error);
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            status: "success".into(),
            data: Some(state.hardware.status().await),
            error: None,
        }),
    )
}

async fn health(AxumState(state): AxumState<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: janus_core::VERSION.to_string(),
        plugins: state.executor.plugin_count(),
    })
}

async fn certify_module(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Json(request): Json<ModuleCertificationRequest>,
) -> (StatusCode, Json<ApiResponse<ModuleCertificationResponse>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .certify_module(
            &account,
            request.module_id,
            request.module_sha256,
            request.status,
            request.required_feature,
            request.notes,
        )
        .await
    {
        Ok(certification) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(ModuleCertificationResponse {
                    module_id: certification.module_id,
                    status: certification.status.as_str().into(),
                    module_sha256: certification.module_sha256,
                }),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn set_engagement_active_route(
    state: ApiState,
    headers: HeaderMap,
    engagement_id: String,
    active: bool,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .set_engagement_active(&account, &engagement_id, active)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}
async fn disable_engagement(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path(engagement_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    set_engagement_active_route(state, headers, engagement_id, false).await
}
async fn enable_engagement(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path(engagement_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    set_engagement_active_route(state, headers, engagement_id, true).await
}

async fn list_engagements(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<janus_core::Engagement>>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.engagements(&account).await {
        Ok(engagements) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(engagements),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn create_engagement(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Json(request): Json<EngagementCreateRequest>,
) -> (StatusCode, Json<ApiResponse<EngagementResponse>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    let scope = EngagementScope {
        approved_assets: request.approved_assets,
        approved_evidence_paths: request.approved_evidence_paths,
        approved_features: request.approved_features,
    };
    match state
        .state_manager
        .create_engagement(
            &account,
            request.authorization_reference,
            request.starts_at,
            request.ends_at,
            scope,
            request.active,
        )
        .await
    {
        Ok(engagement) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(EngagementResponse {
                    engagement_id: engagement.id,
                    organization_id: engagement.organization_id,
                    active: engagement.active,
                }),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn license_status(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<janus_core::LicenseClaims>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.license_claims(&account).await {
        Ok(claims) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(claims),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn import_license(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Json(request): Json<LicenseImportRequest>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .import_license(&account, request.license)
        .await
    {
        Ok(()) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn list_users(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<UserResponse>>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.users(&account).await {
        Ok(users) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(
                    users
                        .into_iter()
                        .map(|user| UserResponse {
                            user_id: user.id,
                            organization_id: user.organization_id,
                            email: user.email,
                            role: user.role.as_str().into(),
                        })
                        .collect(),
                ),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn set_user_active_route(
    state: ApiState,
    headers: HeaderMap,
    user_id: String,
    active: bool,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .set_user_active(&account, &user_id, active)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn disable_user(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    set_user_active_route(state, headers, user_id, false).await
}
async fn enable_user(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    set_user_active_route(state, headers, user_id, true).await
}

#[derive(Deserialize)]
struct PasswordResetRequest {
    password: String,
}

async fn reset_user_password(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(request): Json<PasswordResetRequest>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .reset_user_password(&account, &user_id, request.password)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn create_user(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Json(request): Json<UserCreateRequest>,
) -> (StatusCode, Json<ApiResponse<UserResponse>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state
        .state_manager
        .create_user(&account, request.email, request.password, request.role)
        .await
    {
        Ok(created) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(UserResponse {
                    user_id: created.id,
                    organization_id: created.organization_id,
                    email: created.email,
                    role: created.role.as_str().into(),
                }),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn current_user(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<CurrentUserResponse>>) {
    match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(CurrentUserResponse {
                    user_id: account.id,
                    organization_id: account.organization_id,
                    email: account.email,
                    role: account.role.as_str().into(),
                }),
                error: None,
            }),
        ),
        Err(error) => unauthorized_response(error),
    }
}

async fn logout(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    let token = match bearer_token(&headers) {
        Ok(token) => token,
        Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.logout(&account, token).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(error) => unauthorized_response(error),
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, anyhow::Error> {
    let value = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing authorization bearer token"))?;
    value
        .strip_prefix("Bearer ")
        .ok_or_else(|| anyhow::anyhow!("invalid authorization scheme"))
}

async fn authenticated_account(
    state_manager: &StateManager,
    headers: &HeaderMap,
) -> Result<janus_core::UserAccount> {
    state_manager
        .authenticate_session(bearer_token(headers)?)
        .await
}

fn unauthorized_response<T>(error: anyhow::Error) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            status: "error".into(),
            data: None,
            error: Some(error.to_string()),
        }),
    )
}

async fn login(
    AxumState(state): AxumState<ApiState>,
    Json(request): Json<LoginRequest>,
) -> (StatusCode, Json<ApiResponse<LoginResponse>>) {
    match state
        .state_manager
        .login(request.email, request.password)
        .await
    {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".to_string(),
                data: Some(LoginResponse {
                    session_token: session.token,
                    expires_at: session.expires_at.to_rfc3339(),
                    user_id: session.account.id,
                    organization_id: session.account.organization_id,
                    role: session.account.role.as_str().to_string(),
                }),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn bootstrap(
    AxumState(state): AxumState<ApiState>,
    Json(request): Json<BootstrapRequest>,
) -> (StatusCode, Json<ApiResponse<BootstrapResponse>>) {
    match state
        .state_manager
        .bootstrap_first_admin(request.organization_name, request.email, request.password)
        .await
    {
        Ok((organization, account)) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "success".to_string(),
                data: Some(BootstrapResponse {
                    organization_id: organization.id,
                    user_id: account.id,
                    email: account.email,
                    role: account.role.as_str().to_string(),
                }),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

async fn execute(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ExecuteRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    if !account.role.may_request_execution() {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some("role is not allowed to request execution".to_string()),
            }),
        );
    }
    // The organization-wide add-on switch is persisted in runtime state. Missing
    // state preserves existing installations; an explicit false disables execution.
    let addons_key = StateKey::new("system", "addons_enabled");
    match state.state_manager.get(&account, &addons_key).await {
        Ok(Some(value)) if value == serde_json::Value::Bool(false) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiResponse {
                    status: "error".into(),
                    data: None,
                    error: Some("Janus add-ons are disabled by the organization administrator".into()),
                }),
            );
        }
        Ok(_) => {}
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    status: "error".into(),
                    data: None,
                    error: Some(format!("could not verify add-on setting: {error}")),
                }),
            );
        }
    }

    let module_sha256 = match state.executor.plugin_sha256(&req.plugin) {
        Ok(hash) => hash,
        Err(error) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    status: "error".into(),
                    data: None,
                    error: Some(error.to_string()),
                }),
            )
        }
    };
    if let Err(error) = state
        .state_manager
        .authorize_production_execution(
            &account,
            &req.engagement_id,
            &req.target_asset,
            &req.plugin,
            &module_sha256,
        )
        .await
    {
        let message = error.to_string();
        let _ = state
            .state_manager
            .audit_execution_event(
                &account,
                "EXECUTION_DENIED",
                &req.plugin,
                &req.target_asset,
                Err(&message),
            )
            .await;
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(message),
            }),
        );
    }
    let _ = state
        .state_manager
        .audit_execution_event(
            &account,
            "EXECUTION_AUTHORIZED",
            &req.plugin,
            &req.target_asset,
            Ok(()),
        )
        .await;

    match state.executor.execute(&req.plugin, req.args).await {
        Ok(data) => {
            let _ = state
                .state_manager
                .audit_execution_event(
                    &account,
                    "EXECUTION_SUCCESS",
                    &req.plugin,
                    &req.target_asset,
                    Ok(()),
                )
                .await;
            (
                StatusCode::OK,
                Json(ApiResponse {
                    status: "success".to_string(),
                    data: Some(data),
                    error: None,
                }),
            )
        }
        Err(error) => {
            let message = error.to_string();
            let _ = state
                .state_manager
                .audit_execution_event(
                    &account,
                    "EXECUTION_FAILURE",
                    &req.plugin,
                    &req.target_asset,
                    Err(&message),
                )
                .await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    status: "error".to_string(),
                    data: None,
                    error: Some(message),
                }),
            )
        }
    }
}

async fn list_plugins(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<PluginSummary>>>) {
    if let Err(error) = authenticated_account(&state.state_manager, &headers).await {
        return unauthorized_response(error);
    }
    match state.executor.list_plugins().await {
        Ok(plugins) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".to_string(),
                data: Some(plugins),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some(e.to_string()),
            }),
        ),
    }
}

async fn get_state(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path((namespace, key)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    let state_key = StateKey::new(namespace, key);
    match state.state_manager.get(&account, &state_key).await {
        Ok(Some(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".to_string(),
                data: Some(value),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                status: "not_found".to_string(),
                data: None,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some(e.to_string()),
            }),
        ),
    }
}

async fn set_state(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
    Path((namespace, key)): Path<(String, String)>,
    Json(req): Json<serde_json::Value>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    if !account.role.may_modify_state() {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some("role is not allowed to modify state".into()),
            }),
        );
    }
    let state_key = StateKey::new(namespace, key);
    match state.state_manager.update_state(&account, &state_key, req).await {
        Ok(_) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".to_string(),
                data: Some(()),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".to_string(),
                data: None,
                error: Some(e.to_string()),
            }),
        ),
    }
}

async fn get_audit_logs(
    AxumState(state): AxumState<ApiState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<AuditEntry>>>) {
    let account = match authenticated_account(&state.state_manager, &headers).await {
        Ok(account) => account,
        Err(error) => return unauthorized_response(error),
    };
    match state.state_manager.audit_logs(&account, 100).await {
        Ok(entries) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "success".into(),
                data: Some(entries),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                status: "error".into(),
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}
