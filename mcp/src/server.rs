//! MCP protocol adapter over the transport-free tool and session layers.

use crate::{session::Session, tools};
use base64::Engine;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool as McpTool,
    },
    service::RequestContext,
};
use serde_json::Value;
use std::sync::Mutex;

/// One MCP connection and the rig it keeps open between calls.
#[derive(Default)]
pub struct AnkhimateServer {
    session: Mutex<Session>,
}

impl AnkhimateServer {
    pub fn new() -> Self {
        Self::default()
    }

    fn catalogue() -> Vec<McpTool> {
        tools::all()
            .into_iter()
            .map(|tool| {
                let schema = tool
                    .schema
                    .as_object()
                    .cloned()
                    .expect("tool schemas are objects");
                McpTool::new(tool.name, tool.description, schema)
            })
            .collect()
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> CallToolResult {
        let args = Value::Object(arguments.unwrap_or_default());
        let mut session = match self.session.lock() {
            Ok(session) => session,
            Err(_) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    "the rig session is unavailable after an internal failure",
                )]);
            }
        };

        match tools::call(&mut session, name, &args) {
            Ok(tools::Output::Structured(value)) => CallToolResult::structured(value),
            Ok(tools::Output::StructuredImage {
                structured,
                png,
                width: _,
                height: _,
            }) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                let mut result = CallToolResult::success(vec![
                    rmcp::model::ContentBlock::text(structured.to_string()),
                    rmcp::model::ContentBlock::image(encoded, "image/png"),
                ]);
                result.structured_content = Some(structured);
                result
            }
            Ok(tools::Output::Image { png, width, height }) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                let mut result = CallToolResult::success(vec![rmcp::model::ContentBlock::image(
                    encoded,
                    "image/png",
                )]);
                result.structured_content = Some(
                    serde_json::json!({ "width": width, "height": height, "mime_type": "image/png" }),
                );
                result
            }
            Err(error) => {
                CallToolResult::error(vec![rmcp::model::ContentBlock::text(error.to_string())])
            }
        }
    }
}

impl ServerHandler for AnkhimateServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("ankhimate-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Ankhimate MCP Server")
                    .with_description("Headless rig editing and export for Ankhimate"),
            )
            .with_instructions(
                "Open or create one Ankhimate rig, inspect it, edit it through sandboxed JavaScript, \
                 render frames/contact sheets for visual inspection, then save to a new path or \
                 export it. Never save over the opened source file.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::catalogue()))
    }

    fn get_tool(&self, name: &str) -> Option<McpTool> {
        Self::catalogue().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(self.dispatch(&request.name, request.arguments).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_catalogue_is_the_transport_free_catalogue() {
        let expected: Vec<&str> = tools::all().iter().map(|tool| tool.name).collect();
        let actual: Vec<String> = AnkhimateServer::catalogue()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn protocol_calls_share_one_rig_session() {
        let server = AnkhimateServer::new();
        let created = server.dispatch(
            "new_rig",
            Some(serde_json::Map::from_iter([(
                "name".into(),
                Value::String("hero".into()),
            )])),
        );
        assert_eq!(created.is_error, Some(false));

        let described = server.dispatch("describe_rig", None);
        assert_eq!(
            described.structured_content.unwrap()["project"]["name"],
            "hero"
        );
    }

    #[test]
    fn protocol_packages_rendered_png_as_image_content() {
        let server = AnkhimateServer::new();
        server.dispatch(
            "new_rig",
            Some(serde_json::Map::from_iter([(
                "name".into(),
                Value::String("preview".into()),
            )])),
        );
        let rendered = server.dispatch("render_frame", None);
        assert_eq!(rendered.is_error, Some(false));
        let rmcp::model::ContentBlock::Image(image) = &rendered.content[0] else {
            panic!("render result was not MCP image content");
        };
        assert_eq!(image.mime_type, "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn protocol_packages_structured_data_and_image_together() {
        let server = AnkhimateServer::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.ankh");
        server.dispatch(
            "new_rig",
            Some(serde_json::Map::from_iter([(
                "name".into(),
                Value::String("preview".into()),
            )])),
        );
        server.dispatch(
            "save_rig",
            Some(serde_json::Map::from_iter([(
                "path".into(),
                Value::String(path.to_string_lossy().into_owned()),
            )])),
        );

        let opened = server.dispatch(
            "open_rig",
            Some(serde_json::Map::from_iter([(
                "path".into(),
                Value::String(path.to_string_lossy().into_owned()),
            )])),
        );
        assert_eq!(opened.is_error, Some(false));
        assert_eq!(
            opened.structured_content.unwrap()["assets"]["images"],
            serde_json::json!([])
        );
        assert!(matches!(
            opened.content[0],
            rmcp::model::ContentBlock::Text(_)
        ));
        assert!(matches!(
            opened.content[1],
            rmcp::model::ContentBlock::Image(_)
        ));
    }
}
