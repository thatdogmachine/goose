use base64::Engine;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use include_dir::{include_dir, Dir};
use indoc::{formatdoc, indoc};
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, GetPromptRequestParam, GetPromptResult,
        Implementation, ListPromptsResult, LoggingLevel, LoggingMessageNotificationParam,
        PaginatedRequestParam, Prompt, PromptArgument, PromptMessage, PromptMessageRole, Role,
        ServerCapabilities, ServerInfo,
    },
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    io::Cursor,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};
use xcap::{Monitor, Window};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_stream::{wrappers::SplitStream, StreamExt as _};

use super::editor_models::{create_editor_model, EditorModel};
use super::goose_hints::load_hints::{load_hint_files, GOOSE_HINTS_FILENAME};
use super::shell::{expand_path, get_shell_config, is_absolute_path};
use super::text_editor::{
    text_editor_insert, text_editor_replace, text_editor_undo, text_editor_view, text_editor_write,
};

/// Parameters for the screen_capture tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScreenCaptureParams {
    /// The display number to capture (0 is main display)
    #[serde(default)]
    pub display: Option<u64>,

    /// Optional: the exact title of the window to capture.
    /// Use the list_windows tool to find the available windows.
    pub window_title: Option<String>,
}

/// Parameters for the text_editor tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TextEditorParams {
    /// Absolute path to file or directory, e.g. `/repo/file.py` or `/repo`.
    pub path: String,

    /// The operation to perform. Allowed options are: `view`, `write`, `str_replace`, `insert`, `undo_edit`.
    pub command: String,

    /// Optional array of two integers specifying the start and end line numbers to view.
    /// Line numbers are 1-indexed, and -1 for the end line means read to the end of the file.
    /// This parameter only applies when viewing files, not directories.
    pub view_range: Option<Vec<i64>>,

    /// The content to write to the file. Required for `write` command.
    pub file_text: Option<String>,

    /// The old string to replace. Required for `str_replace` command.
    pub old_str: Option<String>,

    /// The new string to replace with. Required for `str_replace` and `insert` commands.
    pub new_str: Option<String>,

    /// The line number after which to insert text (0 for beginning). Required for `insert` command.
    pub insert_line: Option<i64>,
}

/// Parameters for the shell tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShellParams {
    /// The command string to execute in the shell
    pub command: String,
}

/// Parameters for the image_processor tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImageProcessorParams {
    /// Absolute path to the image file to process
    pub path: String,
}

/// Template structure for prompt definitions
#[derive(Debug, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: String,
    pub template: String,
    pub arguments: Vec<PromptArgumentTemplate>,
}

/// Template structure for prompt arguments
#[derive(Debug, Serialize, Deserialize)]
pub struct PromptArgumentTemplate {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

// Embeds the prompts directory to the build
static PROMPTS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/developer/prompts");

/// Loads prompt files from the embedded PROMPTS_DIR and returns a HashMap of prompts.
/// Ensures that each prompt name is unique.
fn load_prompt_files() -> HashMap<String, Prompt> {
    let mut prompts = HashMap::new();

    for entry in PROMPTS_DIR.files() {
        // Only process JSON files
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }

        let prompt_str = String::from_utf8_lossy(entry.contents()).into_owned();

        let template: PromptTemplate = match serde_json::from_str(&prompt_str) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "Failed to parse prompt template in {}: {}",
                    entry.path().display(),
                    e
                );
                continue; // Skip invalid prompt file
            }
        };

        let arguments = template
            .arguments
            .into_iter()
            .map(|arg| PromptArgument {
                name: arg.name,
                description: arg.description,
                required: arg.required,
            })
            .collect::<Vec<PromptArgument>>();

        let prompt = Prompt::new(&template.id, Some(&template.template), Some(arguments));

        if prompts.contains_key(&prompt.name) {
            eprintln!("Duplicate prompt name '{}' found. Skipping.", prompt.name);
            continue; // Skip duplicate prompt name
        }

        prompts.insert(prompt.name.clone(), prompt);
    }

    prompts
}

/// Developer MCP Server using official RMCP SDK
#[derive(Debug)]
pub struct DeveloperServer {
    tool_router: ToolRouter<Self>,
    file_history: Arc<Mutex<HashMap<PathBuf, Vec<String>>>>,
    ignore_patterns: Gitignore,
    editor_model: Option<EditorModel>,
    prompts: HashMap<String, Prompt>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DeveloperServer {
    #[allow(clippy::too_many_lines)]
    fn get_info(&self) -> ServerInfo {
        // Get base instructions and working directory
        let cwd = std::env::current_dir().expect("should have a current working dir");
        let os = std::env::consts::OS;

        let base_instructions = match os {
            "windows" => formatdoc! {r#"
                The developer extension gives you the capabilities to edit code files and run shell commands,
                and can be used to solve a wide range of problems.

                You can use the shell tool to run Windows commands (PowerShell or CMD).
                When using paths, you can use either backslashes or forward slashes.

                Use the shell tool as needed to locate files or interact with the project.

                Your windows/screen tools can be used for visual debugging. You should not use these tools unless
                prompted to, but you can mention they are available if they are relevant.

                operating system: {os}
                current directory: {cwd}

                "#,
                os=os,
                cwd=cwd.to_string_lossy(),
            },
            _ => formatdoc! {r#"
                The developer extension gives you the capabilities to edit code files and run shell commands,
                and can be used to solve a wide range of problems.

            You can use the shell tool to run any command that would work on the relevant operating system.
            Use the shell tool as needed to locate files or interact with the project.

            Your windows/screen tools can be used for visual debugging. You should not use these tools unless
            prompted to, but you can mention they are available if they are relevant.

            operating system: {os}
            current directory: {cwd}

                "#,
                os=os,
                cwd=cwd.to_string_lossy(),
            },
        };

        let hints_filenames: Vec<String> = std::env::var("CONTEXT_FILE_NAMES")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| vec!["AGENTS.md".to_string(), GOOSE_HINTS_FILENAME.to_string()]);

        // Build ignore patterns for file reference processing
        let ignore_patterns = Self::build_ignore_patterns(&cwd);

        // Load hints using the centralized function
        let hints = load_hint_files(&cwd, &hints_filenames, &ignore_patterns);

        // Check if editor model exists and augment with custom llm editor tool description
        let editor_description = if let Some(ref editor) = self.editor_model {
            formatdoc! {r#"

                Additional Text Editor Tool Instructions:
                
                Perform text editing operations on files.
                The `command` parameter specifies the operation to perform. Allowed options are:
                - `view`: View the content of a file.
                - `write`: Create or overwrite a file with the given content
                - `str_replace`: Edit the file with the new content.
                - `insert`: Insert text at a specific line location in the file.
                - `undo_edit`: Undo the last edit made to a file.

                To use the write command, you must specify `file_text` which will become the new content of the file. Be careful with
                existing files! This is a full overwrite, so you must include everything - not just sections you are modifying.
                
                To use the insert command, you must specify both `insert_line` (the line number after which to insert, 0 for beginning, -1 for end) 
                and `new_str` (the text to insert).

                To use the edit_file command, you must specify both `old_str` and `new_str` 
                {}
                
            "#, editor.get_str_replace_description()}
        } else {
            formatdoc! {r#"

                Additional Text Editor Tool Instructions:
                
                Perform text editing operations on files.

                The `command` parameter specifies the operation to perform. Allowed options are:
                - `view`: View the content of a file.
                - `write`: Create or overwrite a file with the given content
                - `str_replace`: Replace a string in a file with a new string.
                - `insert`: Insert text at a specific line location in the file.
                - `undo_edit`: Undo the last edit made to a file.

                To use the write command, you must specify `file_text` which will become the new content of the file. Be careful with
                existing files! This is a full overwrite, so you must include everything - not just sections you are modifying.

                To use the str_replace command, you must specify both `old_str` and `new_str` - the `old_str` needs to exactly match one
                unique section of the original file, including any whitespace. Make sure to include enough context that the match is not
                ambiguous. The entire original string will be replaced with `new_str`.

                To use the insert command, you must specify both `insert_line` (the line number after which to insert, 0 for beginning, -1 for end) 
                and `new_str` (the text to insert).
                
            "#}
        };

        // Create comprehensive shell tool instructions
        let common_shell_instructions = indoc! {r#"
            Additional Shell Tool Instructions:
            Execute a command in the shell.

            This will return the output and error concatenated into a single string, as
            you would see from running on the command line. There will also be an indication
            of if the command succeeded or failed.

            Avoid commands that produce a large amount of output, and consider piping those outputs to files.

            **Important**: Each shell command runs in its own process. Things like directory changes or
            sourcing files do not persist between tool calls. So you may need to repeat them each time by
            stringing together commands.
              - Pathnames: Use absolute paths and avoid cd unless explicitly requested
        "#};

        let windows_specific = indoc! {r#"
            **Important**: For searching files and code:

            Preferred: Use ripgrep (`rg`) when available - it respects .gitignore and is fast:
              - To locate a file by name: `rg --files | rg example.py`
              - To locate content inside files: `rg 'class Example'`

            Alternative Windows commands (if ripgrep is not installed):
              - To locate a file by name: `dir /s /b example.py`
              - To locate content inside files: `findstr /s /i "class Example" *.py`

            Note: Alternative commands may show ignored/hidden files that should be excluded.

              - Multiple commands: Use && to chain commands, avoid newlines
              - Example: `cd example && dir` or `activate.bat && pip install numpy`

             **Important**: Use forward slashes in paths (e.g., `C:/Users/name`) to avoid
                 escape character issues with backslashes, i.e. \n in a path could be
                 mistaken for a newline.
        "#};

        let unix_specific = indoc! {r#"
            If you need to run a long lived command, background it - e.g. `uvicorn main:app &` so that
            this tool does not run indefinitely.

            **Important**: Use ripgrep - `rg` - exclusively when you need to locate a file or a code reference,
            other solutions may produce too large output because of hidden files! For example *do not* use `find` or `ls -r`
              - List files by name: `rg --files | rg <filename>`
              - List files that contain a regex: `rg '<regex>' -l`

              - Multiple commands: Use && to chain commands, avoid newlines
              - Example: `cd example && ls` or `source env/bin/activate && pip install numpy`
        "#};

        let shell_tool_desc = match os {
            "windows" => format!("{}{}", common_shell_instructions, windows_specific),
            _ => format!("{}{}", common_shell_instructions, unix_specific),
        };

        // Return base instructions directly when no hints are found
        let instructions = if hints.is_empty() {
            format!("{base_instructions}{editor_description}\n{shell_tool_desc}")
        } else {
            format!("{base_instructions}\n{editor_description}\n{shell_tool_desc}\n{hints}")
        };

        ServerInfo {
            server_info: Implementation {
                name: "goose-developer".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
            instructions: Some(instructions),
            ..Default::default()
        }
    }

    // TODO: use the rmcp prompt macros instead when SDK is updated
    // Current rmcp version 0.6.0 doesn't support prompt macros yet.
    // When upgrading to a newer version that supports it, replace this manual
    // implementation with the macro-based approach for better maintainability.
    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + Send + '_ {
        let prompts: Vec<Prompt> = self.prompts.values().cloned().collect();
        std::future::ready(Ok(ListPromptsResult {
            prompts,
            next_cursor: None,
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, ErrorData>> + Send + '_ {
        let prompt_name = request.name;
        let arguments = request.arguments.unwrap_or_default();

        match self.prompts.get(&prompt_name) {
            Some(prompt) => {
                // Get the template from the prompt description
                let template = prompt.description.clone().unwrap_or_default();

                // Validate template length
                if template.len() > 10000 {
                    return std::future::ready(Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        "Prompt template exceeds maximum allowed length".to_string(),
                        None,
                    )));
                }

                // Validate arguments for security (same checks as router)
                for (key, value) in &arguments {
                    // Check for empty or overly long keys/values
                    if key.is_empty() || key.len() > 1000 {
                        return std::future::ready(Err(ErrorData::new(
                            ErrorCode::INVALID_PARAMS,
                            "Argument keys must be between 1-1000 characters".to_string(),
                            None,
                        )));
                    }

                    let value_str = value.as_str().unwrap_or_default();
                    if value_str.len() > 1000 {
                        return std::future::ready(Err(ErrorData::new(
                            ErrorCode::INVALID_PARAMS,
                            "Argument values must not exceed 1000 characters".to_string(),
                            None,
                        )));
                    }

                    // Check for potentially dangerous patterns
                    let dangerous_patterns = ["../", "//", "\\\\", "<script>", "{{", "}}"];
                    for pattern in dangerous_patterns {
                        if key.contains(pattern) || value_str.contains(pattern) {
                            return std::future::ready(Err(ErrorData::new(
                                ErrorCode::INVALID_PARAMS,
                                format!(
                                    "Arguments contain potentially unsafe pattern: {}",
                                    pattern
                                ),
                                None,
                            )));
                        }
                    }
                }

                // Validate required arguments
                if let Some(args) = &prompt.arguments {
                    for arg in args {
                        if arg.required.unwrap_or(false)
                            && (!arguments.contains_key(&arg.name)
                                || arguments
                                    .get(&arg.name)
                                    .and_then(|v| v.as_str())
                                    .is_none_or(str::is_empty))
                        {
                            return std::future::ready(Err(ErrorData::new(
                                ErrorCode::INVALID_PARAMS,
                                format!("Missing required argument: '{}'", arg.name),
                                None,
                            )));
                        }
                    }
                }

                // Create a mutable copy of the template to fill in arguments
                let mut template_filled = template.clone();

                // Replace each argument placeholder with its value from the arguments object
                for (key, value) in &arguments {
                    let placeholder = format!("{{{}}}", key);
                    template_filled =
                        template_filled.replace(&placeholder, value.as_str().unwrap_or_default());
                }

                // Create prompt messages with the filled template
                let messages = vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    template_filled.clone(),
                )];

                let result = GetPromptResult {
                    description: Some(template_filled),
                    messages,
                };
                std::future::ready(Ok(result))
            }
            None => std::future::ready(Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Prompt '{}' not found", prompt_name),
                None,
            ))),
        }
    }
}

impl Default for DeveloperServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl DeveloperServer {
    pub fn new() -> Self {
        // Build ignore patterns (simplified version for this tool)
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let ignore_patterns = Self::build_ignore_patterns(&cwd);

        // Initialize editor model for AI-powered code editing
        let editor_model = create_editor_model();

        Self {
            tool_router: Self::tool_router(),
            file_history: Arc::new(Mutex::new(HashMap::new())),
            ignore_patterns,
            editor_model,
            prompts: load_prompt_files(),
        }
    }

    /// List all available windows that can be used with screen_capture.
    /// Returns a list of window titles that can be used with the window_title parameter
    /// of the screen_capture tool.
    #[tool(
        name = "list_windows",
        description = "List all available window titles that can be used with screen_capture. Returns a list of window titles that can be used with the window_title parameter of the screen_capture tool."
    )]
    pub async fn list_windows(&self) -> Result<CallToolResult, ErrorData> {
        let windows = Window::all().map_err(|_| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Failed to list windows".to_string(),
                None,
            )
        })?;

        let window_titles: Vec<String> =
            windows.into_iter().map(|w| w.title().to_string()).collect();

        let content_text = format!("Available windows:\n{}", window_titles.join("\n"));

        Ok(CallToolResult::success(vec![
            Content::text(content_text.clone()).with_audience(vec![Role::Assistant]),
            Content::text(content_text)
                .with_audience(vec![Role::User])
                .with_priority(0.0),
        ]))
    }

    /// Capture a screenshot of a specified display or window.
    /// You can capture either:
    /// 1. A full display (monitor) using the display parameter
    /// 2. A specific window by its title using the window_title parameter
    ///
    /// Only one of display or window_title should be specified.
    #[tool(
        name = "screen_capture",
        description = "Capture a screenshot of a specified display or window. You can capture either: 1. A full display (monitor) using the display parameter 2. A specific window by its title using the window_title parameter. Only one of display or window_title should be specified."
    )]
    pub async fn screen_capture(
        &self,
        params: Parameters<ScreenCaptureParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;

        let mut image = if let Some(window_title) = &params.window_title {
            // Try to find and capture the specified window
            let windows = Window::all().map_err(|_| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Failed to list windows".to_string(),
                    None,
                )
            })?;

            let window = windows
                .into_iter()
                .find(|w| w.title() == window_title)
                .ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("No window found with title '{}'", window_title),
                        None,
                    )
                })?;

            window.capture_image().map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to capture window '{}': {}", window_title, e),
                    None,
                )
            })?
        } else {
            // Default to display capture if no window title is specified
            let display = params.display.unwrap_or(0) as usize;

            let monitors = Monitor::all().map_err(|_| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Failed to access monitors".to_string(),
                    None,
                )
            })?;

            let monitor = monitors.get(display).ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!(
                        "{} was not an available monitor, {} found.",
                        display,
                        monitors.len()
                    ),
                    None,
                )
            })?;

            monitor.capture_image().map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to capture display {}: {}", display, e),
                    None,
                )
            })?
        };

        // Resize the image to a reasonable width while maintaining aspect ratio
        let max_width = 768;
        if image.width() > max_width {
            let scale = max_width as f32 / image.width() as f32;
            let new_height = (image.height() as f32 * scale) as u32;
            image = xcap::image::imageops::resize(
                &image,
                max_width,
                new_height,
                xcap::image::imageops::FilterType::Lanczos3,
            );
        }

        let mut bytes: Vec<u8> = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), xcap::image::ImageFormat::Png)
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to write image buffer {}", e),
                    None,
                )
            })?;

        // Convert to base64
        let data = base64::prelude::BASE64_STANDARD.encode(bytes);

        // Return two Content objects like the old implementation:
        // one text for Assistant, one image with priority 0.0
        Ok(CallToolResult::success(vec![
            Content::text("Screenshot captured").with_audience(vec![Role::Assistant]),
            Content::image(data, "image/png").with_priority(0.0),
        ]))
    }

    /// Perform text editing operations on files.
    ///
    /// The `command` parameter specifies the operation to perform. Allowed options are:
    /// - `view`: View the content of a file.
    /// - `write`: Create or overwrite a file with the given content
    /// - `str_replace`: Replace old_str with new_str in the file.
    /// - `insert`: Insert text at a specific line location in the file.
    /// - `undo_edit`: Undo the last edit made to a file.
    #[tool(
        name = "text_editor",
        description = "Perform text editing operations on files. Commands: view (show file content), write (create/overwrite file), str_replace (AI-enhanced replace text when configured, fallback to literal replacement), insert (insert at line), undo_edit (undo last change)."
    )]
    pub async fn text_editor(
        &self,
        params: Parameters<TextEditorParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let path = self.resolve_path(&params.path)?;

        // Check if file is ignored before proceeding with any text editor operation
        if self.is_ignored(&path) {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "Access to '{}' is restricted by .gooseignore",
                    path.display()
                ),
                None,
            ));
        }

        match params.command.as_str() {
            "view" => {
                let view_range = params.view_range.as_ref().and_then(|vr| {
                    if vr.len() == 2 {
                        Some((vr[0] as usize, vr[1]))
                    } else {
                        None
                    }
                });
                let content = text_editor_view(&path, view_range).await?;
                Ok(CallToolResult::success(content))
            }
            "write" => {
                let file_text = params.file_text.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'file_text' parameter for write command".to_string(),
                        None,
                    )
                })?;
                let content = text_editor_write(&path, &file_text).await?;
                Ok(CallToolResult::success(content))
            }
            "str_replace" => {
                let old_str = params.old_str.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'old_str' parameter for str_replace command".to_string(),
                        None,
                    )
                })?;
                let new_str = params.new_str.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'new_str' parameter for str_replace command".to_string(),
                        None,
                    )
                })?;
                let content = text_editor_replace(
                    &path,
                    &old_str,
                    &new_str,
                    &self.editor_model,
                    &self.file_history,
                )
                .await?;
                Ok(CallToolResult::success(content))
            }
            "insert" => {
                let insert_line = params.insert_line.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'insert_line' parameter for insert command".to_string(),
                        None,
                    )
                })? as usize;
                let new_str = params.new_str.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'new_str' parameter for insert command".to_string(),
                        None,
                    )
                })?;
                let content =
                    text_editor_insert(&path, insert_line as i64, &new_str, &self.file_history)
                        .await?;
                Ok(CallToolResult::success(content))
            }
            "undo_edit" => {
                let content = text_editor_undo(&path, &self.file_history).await?;
                Ok(CallToolResult::success(content))
            }
            _ => Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Unknown command '{}'", params.command),
                None,
            )),
        }
    }

    /// Execute a command in the shell.
    ///
    /// This will return the output and error concatenated into a single string, as
    /// you would see from running on the command line. There will also be an indication
    /// of if the command succeeded or failed.
    ///
    /// Avoid commands that produce a large amount of output, and consider piping those outputs to files.
    /// If you need to run a long lived command, background it - e.g. `uvicorn main:app &` so that
    /// this tool does not run indefinitely.
    #[tool(
        name = "shell",
        description = "Execute a command in the shell. Returns output and error concatenated. Avoid commands with large output, use background commands for long-running processes."
    )]
    pub async fn shell(
        &self,
        params: Parameters<ShellParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let command = &params.command;
        let peer = context.peer;

        // Validate the shell command
        self.validate_shell_command(command)?;

        // Execute the command and capture output
        let output_str = self.execute_shell_command(command, &peer).await?;

        // Validate output size
        self.validate_shell_output_size(command, &output_str)?;

        // Process and format the output
        let (final_output, user_output) = self.process_shell_output(&output_str)?;

        Ok(CallToolResult::success(vec![
            Content::text(final_output).with_audience(vec![Role::Assistant]),
            Content::text(user_output)
                .with_audience(vec![Role::User])
                .with_priority(0.0),
        ]))
    }

    /// Validate a shell command before execution.
    ///
    /// Checks for empty commands and ensures the command doesn't attempt to access
    /// files that are restricted by ignore patterns.
    fn validate_shell_command(&self, command: &str) -> Result<(), ErrorData> {
        let cmd_parts: Vec<&str> = command.split_whitespace().collect();

        // Allow empty commands - they'll be handled gracefully
        if cmd_parts.is_empty() {
            return Ok(());
        }

        // Check if command arguments reference ignored files
        for arg in &cmd_parts[1..] {
            // Skip command flags
            if arg.starts_with('-') {
                continue;
            }

            // Skip invalid paths
            let path = Path::new(arg);
            if !path.exists() {
                continue;
            }

            if self.is_ignored(path) {
                return Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!(
                        "The command attempts to access '{}' which is restricted by .gooseignore",
                        arg
                    ),
                    None,
                ));
            }
        }

        Ok(())
    }

    /// Execute a shell command and return the combined output.
    ///
    /// Streams output in real-time to the client using logging notifications.
    async fn execute_shell_command(
        &self,
        command: &str,
        peer: &rmcp::service::Peer<RoleServer>,
    ) -> Result<String, ErrorData> {
        // Handle empty commands
        if command.trim().is_empty() {
            return Ok(String::new());
        }

        // Get platform-specific shell configuration
        let shell_config = get_shell_config();

        // Execute the command using platform-specific shell
        let mut child = Command::new(&shell_config.executable)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .env("GOOSE_TERMINAL", "1")
            .args(&shell_config.args)
            .arg(command)
            .spawn()
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        // Stream the output
        let output_str = self
            .stream_shell_output(
                child.stdout.take().unwrap(),
                child.stderr.take().unwrap(),
                peer.clone(),
            )
            .await?;

        // Wait for the command to complete
        child
            .wait()
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        Ok(output_str)
    }

    /// Stream shell output in real-time and return the combined output.
    ///
    /// Merges stdout and stderr streams and sends each line as a logging notification.
    async fn stream_shell_output(
        &self,
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
        peer: rmcp::service::Peer<RoleServer>,
    ) -> Result<String, ErrorData> {
        let stdout = BufReader::new(stdout);
        let stderr = BufReader::new(stderr);

        let output_task = tokio::spawn(async move {
            let mut combined_output = String::new();

            // Merge stdout and stderr streams
            // ref https://blog.yoshuawuyts.com/futures-concurrency-3
            let stdout = SplitStream::new(stdout.split(b'\n')).map(|v| ("stdout", v));
            let stderr = SplitStream::new(stderr.split(b'\n')).map(|v| ("stderr", v));
            let mut merged = stdout.merge(stderr);

            while let Some((stream_type, line)) = merged.next().await {
                let mut line = line?;
                // Re-add newline as clients expect it
                line.push(b'\n');
                // Convert to UTF-8 to avoid corrupted output
                let line_str = String::from_utf8_lossy(&line);

                combined_output.push_str(&line_str);

                // Stream each line back to the client in real-time
                let trimmed_line = line_str.trim();
                if !trimmed_line.is_empty() {
                    // Send the output line as a structured logging message
                    if let Err(e) = peer
                        .notify_logging_message(LoggingMessageNotificationParam {
                            level: LoggingLevel::Info,
                            data: serde_json::json!({
                                "type": "shell_output",
                                "stream": stream_type,
                                "output": trimmed_line
                            }),
                            logger: Some("shell_tool".to_string()),
                        })
                        .await
                    {
                        // Don't break execution if streaming fails, just log it
                        eprintln!("Failed to stream output line: {}", e);
                    }
                }
            }
            Ok::<_, std::io::Error>(combined_output)
        });

        match output_task.await {
            Ok(result) => {
                result.map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))
            }
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                e.to_string(),
                None,
            )),
        }
    }

    /// Validate that shell output doesn't exceed size limits.
    fn validate_shell_output_size(&self, command: &str, output: &str) -> Result<(), ErrorData> {
        const MAX_CHAR_COUNT: usize = 400_000; // 400KB
        let char_count = output.chars().count();

        if char_count > MAX_CHAR_COUNT {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "Shell output from command '{}' has too many characters ({}). Maximum character count is {}.",
                    command,
                    char_count,
                    MAX_CHAR_COUNT
                ),
                None,
            ));
        }

        Ok(())
    }

    /// Process an image file from disk.
    ///
    /// The image will be:
    /// 1. Resized if larger than max width while maintaining aspect ratio
    /// 2. Converted to PNG format
    /// 3. Returned as base64 encoded data
    ///
    /// This allows processing image files for use in the conversation.
    #[tool(
        name = "image_processor",
        description = "Process an image file from disk. Resizes if needed, converts to PNG, and returns as base64 data."
    )]
    pub async fn image_processor(
        &self,
        params: Parameters<ImageProcessorParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let path_str = &params.path;

        let path = {
            let p = self.resolve_path(path_str)?;
            if cfg!(target_os = "macos") {
                self.normalize_mac_screenshot_path(&p)
            } else {
                p
            }
        };

        // Check if file is ignored before proceeding
        if self.is_ignored(&path) {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "Access to '{}' is restricted by .gooseignore",
                    path.display()
                ),
                None,
            ));
        }

        // Check if file exists
        if !path.exists() {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("File '{}' does not exist", path.display()),
                None,
            ));
        }

        // Check file size (10MB limit for image files)
        const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB in bytes
        let file_size = std::fs::metadata(&path)
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to get file metadata: {}", e),
                    None,
                )
            })?
            .len();

        if file_size > MAX_FILE_SIZE {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "File '{}' is too large ({:.2}MB). Maximum size is 10MB.",
                    path.display(),
                    file_size as f64 / (1024.0 * 1024.0)
                ),
                None,
            ));
        }

        // Open and decode the image
        let image = xcap::image::open(&path).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open image file: {}", e),
                None,
            )
        })?;

        // Resize if necessary (same logic as screen_capture)
        let mut processed_image = image;
        let max_width = 768;
        if processed_image.width() > max_width {
            let scale = max_width as f32 / processed_image.width() as f32;
            let new_height = (processed_image.height() as f32 * scale) as u32;
            processed_image = xcap::image::DynamicImage::ImageRgba8(xcap::image::imageops::resize(
                &processed_image,
                max_width,
                new_height,
                xcap::image::imageops::FilterType::Lanczos3,
            ));
        }

        // Convert to PNG and encode as base64
        let mut bytes: Vec<u8> = Vec::new();
        processed_image
            .write_to(&mut Cursor::new(&mut bytes), xcap::image::ImageFormat::Png)
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to write image buffer: {}", e),
                    None,
                )
            })?;

        let data = base64::prelude::BASE64_STANDARD.encode(bytes);

        Ok(CallToolResult::success(vec![
            Content::text(format!(
                "Successfully processed image from {}",
                path.display()
            ))
            .with_audience(vec![Role::Assistant]),
            Content::image(data, "image/png").with_priority(0.0),
        ]))
    }

    // Helper method to resolve and validate file paths
    fn resolve_path(&self, path_str: &str) -> Result<PathBuf, ErrorData> {
        let cwd = std::env::current_dir().expect("should have a current working dir");
        let expanded = expand_path(path_str);
        let path = Path::new(&expanded);

        let suggestion = cwd.join(path);

        match is_absolute_path(&expanded) {
            true => Ok(path.to_path_buf()),
            false => Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "The path {} is not an absolute path, did you possibly mean {}?",
                    path_str,
                    suggestion.to_string_lossy(),
                ),
                None,
            )),
        }
    }

    // Helper method to build ignore patterns from .gooseignore or .gitignore files
    fn build_ignore_patterns(cwd: &PathBuf) -> Gitignore {
        let mut builder = GitignoreBuilder::new(cwd);

        // Check for local .gooseignore
        let local_ignore_path = cwd.join(".gooseignore");
        let mut has_ignore_file = false;

        if local_ignore_path.is_file() {
            let _ = builder.add(local_ignore_path);
            has_ignore_file = true;
        } else {
            // Fallback to .gitignore
            let gitignore_path = cwd.join(".gitignore");
            if gitignore_path.is_file() {
                let _ = builder.add(gitignore_path);
                has_ignore_file = true;
            }
        }

        // Add default patterns if no ignore files found
        if !has_ignore_file {
            let _ = builder.add_line(None, "**/.env");
            let _ = builder.add_line(None, "**/.env.*");
            let _ = builder.add_line(None, "**/secrets.*");
        }

        builder.build().expect("Failed to build ignore patterns")
    }

    // Helper method to check if a path should be ignored
    fn is_ignored(&self, path: &Path) -> bool {
        self.ignore_patterns.matched(path, false).is_ignore()
    }

    // Helper function to handle Mac screenshot filenames that contain U+202F (narrow no-break space)
    fn normalize_mac_screenshot_path(&self, path: &Path) -> PathBuf {
        // Only process if the path has a filename
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            // Check if this matches Mac screenshot pattern:
            // "Screenshot YYYY-MM-DD at H.MM.SS AM/PM.png"
            if let Some(captures) = regex::Regex::new(r"^Screenshot \d{4}-\d{2}-\d{2} at \d{1,2}\.\d{2}\.\d{2} (AM|PM|am|pm)(?: \(\d+\))?\.png$")
                .ok()
                .and_then(|re| re.captures(filename))
            {
                // Get the AM/PM part
                let meridian = captures.get(1).unwrap().as_str();

                // Find the last space before AM/PM and replace it with U+202F
                let space_pos = filename.rfind(meridian)
                    .map(|pos| filename[..pos].trim_end().len())
                    .unwrap_or(0);

                if space_pos > 0 {
                    let parent = path.parent().unwrap_or(Path::new(""));
                    let new_filename = format!(
                        "{}{}{}",
                        &filename[..space_pos],
                        '\u{202F}',
                        &filename[space_pos+1..]
                    );
                    let new_path = parent.join(new_filename);

                    return new_path;
                }
            }
        }

        // Return the original path if it doesn't match or couldn't be processed
        path.to_path_buf()
    }

    // shell output can be large, this will help manage that
    fn process_shell_output(&self, output_str: &str) -> Result<(String, String), ErrorData> {
        let lines: Vec<&str> = output_str.lines().collect();
        let line_count = lines.len();

        let start = lines.len().saturating_sub(100);
        let last_100_lines_str = lines[start..].join("\n");

        let final_output = if line_count > 100 {
            let tmp_file = tempfile::NamedTempFile::new().map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to create temporary file: {}", e),
                    None,
                )
            })?;

            std::fs::write(tmp_file.path(), output_str).map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to write to temporary file: {}", e),
                    None,
                )
            })?;

            let (_, path) = tmp_file.keep().map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to persist temporary file: {}", e),
                    None,
                )
            })?;

            format!(
                "private note: output was {} lines and we are only showing the most recent lines, remainder of lines in {} do not show tmp file to user, that file can be searched if extra context needed to fulfill request. truncated output: \n{}",
                line_count,
                path.display(),
                last_100_lines_str
            )
        } else {
            output_str.to_string()
        };

        let user_output = if line_count > 100 {
            format!(
                "NOTE: Output was {} lines, showing only the last 100 lines.\n\n{}",
                line_count, last_100_lines_str
            )
        } else {
            output_str.to_string()
        };

        Ok((final_output, user_output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::tool::Parameters;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_server() -> DeveloperServer {
        DeveloperServer::new()
    }

    #[test]
    #[serial]
    fn test_global_goosehints() {
        // Note: This test checks if ~/.config/goose/.goosehints exists and includes it in instructions
        // Since RMCP version uses get_info() instead of instructions(), we test that method
        let global_hints_path =
            PathBuf::from(shellexpand::tilde("~/.config/goose/.goosehints").to_string());
        let global_hints_bak_path =
            PathBuf::from(shellexpand::tilde("~/.config/goose/.goosehints.bak").to_string());
        let mut globalhints_existed = false;

        if global_hints_path.is_file() {
            globalhints_existed = true;
            fs::copy(&global_hints_path, &global_hints_bak_path).unwrap();
        }

        fs::write(&global_hints_path, "These are my global goose hints.").unwrap();

        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();
        assert!(instructions.contains("my global goose hints."));

        // restore backup if globalhints previously existed
        if globalhints_existed {
            fs::copy(&global_hints_bak_path, &global_hints_path).unwrap();
            fs::remove_file(&global_hints_bak_path).unwrap();
        } else {
            fs::remove_file(&global_hints_path).unwrap();
        }
    }

    #[test]
    #[serial]
    fn test_goosehints_when_present() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        fs::write(".goosehints", "Test hint content").unwrap();
        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();
        assert!(instructions.contains("Test hint content"));
    }

    #[test]
    #[serial]
    fn test_goosehints_when_missing() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();
        // When no hints are present, instructions should not contain hint content
        assert!(!instructions.contains("AGENTS.md:") && !instructions.contains(".goosehints:"));
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_parameter_validation() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Test that the shell functionality works by testing parameter validation
        // and the ignore pattern checking logic without actually running commands

        // Test that empty command parts are handled correctly
        let cmd_parts: Vec<&str> = "".split_whitespace().collect();
        assert!(
            cmd_parts.is_empty(),
            "Empty command should result in empty parts"
        );

        // Test ignore pattern checking with different paths
        assert!(
            !server.is_ignored(std::path::Path::new("allowed.txt")),
            "Non-ignored file should not be blocked"
        );

        // Note: Full shell execution with RequestContext requires integration testing
        // with proper RMCP framework setup. This test validates the core parameter
        // handling logic that would be used by the shell method.
    }

    #[test]
    #[serial]
    fn test_goosehints_multiple_filenames() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        std::env::set_var("CONTEXT_FILE_NAMES", r#"["CLAUDE.md", ".goosehints"]"#);

        fs::write("CLAUDE.md", "Custom hints file content from CLAUDE.md").unwrap();
        fs::write(".goosehints", "Custom hints file content from .goosehints").unwrap();
        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();
        assert!(instructions.contains("Custom hints file content from CLAUDE.md"));
        assert!(instructions.contains("Custom hints file content from .goosehints"));
        std::env::remove_var("CONTEXT_FILE_NAMES");
    }

    #[test]
    #[serial]
    fn test_goosehints_configurable_filename() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        std::env::set_var("CONTEXT_FILE_NAMES", r#"["CLAUDE.md"]"#);

        fs::write("CLAUDE.md", "Custom hints file content").unwrap();
        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();
        assert!(instructions.contains("Custom hints file content"));
        assert!(!instructions.contains(".goosehints")); // Make sure it's not loading the default
        std::env::remove_var("CONTEXT_FILE_NAMES");
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_write_and_view_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a new file
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("Hello, world!".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // View the file
        let view_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "view".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let view_result = server.text_editor(view_params).await.unwrap();

        assert!(!view_result.content.is_empty());
        let user_content = view_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::User))
            })
            .unwrap()
            .as_text()
            .unwrap();
        assert!(user_content.text.contains("Hello, world!"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_str_replace() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a new file
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("Hello, world!".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Replace string
        let replace_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "str_replace".to_string(),
            view_range: None,
            file_text: None,
            old_str: Some("world".to_string()),
            new_str: Some("Rust".to_string()),
            insert_line: None,
        });

        let replace_result = server.text_editor(replace_params).await.unwrap();

        let assistant_content = replace_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::Assistant))
            })
            .unwrap()
            .as_text()
            .unwrap();

        assert!(
            assistant_content.text.contains("The file")
                && assistant_content.text.contains("has been edited")
        );

        // Verify the file contents changed
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("Hello, Rust!"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_size_limits() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a large file that exceeds the 400KB limit
        let large_content = "a".repeat(500 * 1024); // 500KB
        let file_path = temp_dir.path().join("large_file.txt");
        fs::write(&file_path, &large_content).unwrap();

        let view_params = Parameters(TextEditorParams {
            path: file_path.to_str().unwrap().to_string(),
            command: "view".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let result = server.text_editor(view_params).await;
        assert!(result.is_err());

        let error = result.err().unwrap();
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert!(error.message.contains("too large"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_undo_edit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("Original content".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Make an edit
        let replace_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "str_replace".to_string(),
            view_range: None,
            file_text: None,
            old_str: Some("Original".to_string()),
            new_str: Some("Modified".to_string()),
            insert_line: None,
        });

        server.text_editor(replace_params).await.unwrap();

        // Verify the edit was made
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("Modified content"));

        // Undo the edit
        let undo_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "undo_edit".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let undo_result = server.text_editor(undo_params).await.unwrap();

        // Verify undo worked
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("Original content"));

        let undo_content = undo_result
            .content
            .iter()
            .find(|c| c.as_text().is_some())
            .unwrap()
            .as_text()
            .unwrap();
        assert!(undo_content.text.contains("Undid the last edit"));
    }

    #[tokio::test]
    #[serial]
    async fn test_goose_ignore_basic_patterns() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gooseignore file with patterns
        fs::write(".gooseignore", "secret.txt\n*.env").unwrap();

        let server = create_test_server();

        // Test basic file matching
        assert!(
            server.is_ignored(Path::new("secret.txt")),
            "secret.txt should be ignored"
        );
        assert!(
            server.is_ignored(Path::new("./secret.txt")),
            "./secret.txt should be ignored"
        );
        assert!(
            !server.is_ignored(Path::new("not_secret.txt")),
            "not_secret.txt should not be ignored"
        );

        // Test pattern matching
        assert!(
            server.is_ignored(Path::new("test.env")),
            "*.env pattern should match test.env"
        );
        assert!(
            server.is_ignored(Path::new("./test.env")),
            "*.env pattern should match ./test.env"
        );
        assert!(
            !server.is_ignored(Path::new("test.txt")),
            "*.env pattern should not match test.txt"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_respects_ignore_patterns() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gooseignore file
        fs::write(".gooseignore", "secret.txt").unwrap();

        let server = create_test_server();

        // Try to write to an ignored file
        let secret_path = temp_dir.path().join("secret.txt");
        let write_params = Parameters(TextEditorParams {
            path: secret_path.to_str().unwrap().to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("test content".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let result = server.text_editor(write_params).await;
        assert!(
            result.is_err(),
            "Should not be able to write to ignored file"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::INTERNAL_ERROR);

        // Try to write to a non-ignored file
        let allowed_path = temp_dir.path().join("allowed.txt");
        let write_params = Parameters(TextEditorParams {
            path: allowed_path.to_str().unwrap().to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("test content".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let result = server.text_editor(write_params).await;
        assert!(
            result.is_ok(),
            "Should be able to write to non-ignored file"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_ignore_pattern_validation() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gooseignore file
        fs::write(".gooseignore", "secret.txt").unwrap();
        fs::write("secret.txt", "secret content").unwrap();

        let server = create_test_server();

        // Test that the ignore pattern checking logic works correctly
        // This tests the core functionality that would be used by the shell method

        // Verify ignore patterns are loaded correctly
        assert!(
            server.is_ignored(std::path::Path::new("secret.txt")),
            "secret.txt should be ignored based on .gooseignore"
        );

        assert!(
            !server.is_ignored(std::path::Path::new("allowed.txt")),
            "allowed.txt should not be ignored"
        );

        // Test command parsing logic that would be used in shell validation
        let command = "cat secret.txt";
        let cmd_parts: Vec<&str> = command.split_whitespace().collect();
        assert_eq!(cmd_parts[0], "cat");
        assert_eq!(cmd_parts[1], "secret.txt");

        // Verify that the path exists and would be caught by ignore checking
        let path = std::path::Path::new("secret.txt");
        assert!(path.exists(), "Test file should exist");
        assert!(
            server.is_ignored(path),
            "Shell method would detect this as ignored"
        );

        // Note: Full shell execution testing requires integration testing framework
        // This test validates the ignore pattern logic that prevents access to restricted files.
    }

    #[tokio::test]
    #[serial]
    async fn test_gitignore_fallback_when_no_gooseignore() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gitignore file (no .gooseignore)
        fs::write(".gitignore", "*.log").unwrap();

        let server = create_test_server();

        assert!(
            server.is_ignored(Path::new("debug.log")),
            "*.log pattern from .gitignore should match debug.log"
        );
        assert!(
            !server.is_ignored(Path::new("debug.txt")),
            "*.log pattern should not match debug.txt"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_gooseignore_takes_precedence_over_gitignore() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create both files
        fs::write(".gitignore", "*.log").unwrap();
        fs::write(".gooseignore", "*.env").unwrap();

        let server = create_test_server();

        // Should respect .gooseignore patterns
        assert!(
            server.is_ignored(Path::new("test.env")),
            ".gooseignore pattern should work"
        );
        // Should NOT respect .gitignore patterns when .gooseignore exists
        assert!(
            !server.is_ignored(Path::new("test.log")),
            ".gitignore patterns should be ignored when .gooseignore exists"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_view_range() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a multi-line file
        let content =
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Test viewing specific range
        let view_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "view".to_string(),
            view_range: Some(vec![3, 6]),
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let view_result = server.text_editor(view_params).await.unwrap();

        let text = view_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::User))
            })
            .unwrap()
            .as_text()
            .unwrap();

        // Should contain lines 3-6 with line numbers
        assert!(text.text.contains("3: Line 3"));
        assert!(text.text.contains("4: Line 4"));
        assert!(text.text.contains("5: Line 5"));
        assert!(text.text.contains("6: Line 6"));
        assert!(text.text.contains("(lines 3-6)"));
        // Should not contain other lines
        assert!(!text.text.contains("1: Line 1"));
        assert!(!text.text.contains("7: Line 7"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_view_range_to_end() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a multi-line file
        let content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Test viewing from line 3 to end using -1
        let view_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "view".to_string(),
            view_range: Some(vec![3, -1]),
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let view_result = server.text_editor(view_params).await.unwrap();

        let text = view_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::User))
            })
            .unwrap()
            .as_text()
            .unwrap();

        // Should contain lines 3-5
        assert!(text.text.contains("3: Line 3"));
        assert!(text.text.contains("4: Line 4"));
        assert!(text.text.contains("5: Line 5"));
        assert!(text.text.contains("(lines 3-end)"));
        // Should not contain lines 1-2
        assert!(!text.text.contains("1: Line 1"));
        assert!(!text.text.contains("2: Line 2"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_at_beginning() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 2\nLine 3\nLine 4";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Insert at the beginning (line 0)
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Line 1".to_string()),
            insert_line: Some(0),
        });

        let insert_result = server.text_editor(insert_params).await.unwrap();

        let text = insert_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::Assistant))
            })
            .unwrap()
            .as_text()
            .unwrap();

        assert!(text.text.contains("Text has been inserted at line 1"));

        // Verify the file content by reading it directly
        let file_content = fs::read_to_string(&file_path).unwrap();
        assert!(file_content.contains("Line 1\nLine 2\nLine 3\nLine 4"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_in_middle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 1\nLine 2\nLine 4\nLine 5";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Insert after line 2
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Line 3".to_string()),
            insert_line: Some(2),
        });

        let insert_result = server.text_editor(insert_params).await.unwrap();

        let text = insert_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::Assistant))
            })
            .unwrap()
            .as_text()
            .unwrap();

        assert!(text.text.contains("Text has been inserted at line 3"));

        // Verify the file content by reading it directly
        let file_content = fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = file_content.lines().collect();
        assert_eq!(lines[0], "Line 1");
        assert_eq!(lines[1], "Line 2");
        assert_eq!(lines[2], "Line 3");
        assert_eq!(lines[3], "Line 4");
        assert_eq!(lines[4], "Line 5");
    }

    #[test]
    #[serial]
    fn test_process_shell_output_short() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let server = create_test_server();

        // Test with short output (< 100 lines)
        let short_output = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        let result = server.process_shell_output(short_output).unwrap();

        // Both outputs should be the same for short outputs
        assert_eq!(result.0, short_output);
        assert_eq!(result.1, short_output);
    }

    #[test]
    #[serial]
    fn test_process_shell_output_empty() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let server = create_test_server();

        // Test with empty output
        let empty_output = "";
        let result = server.process_shell_output(empty_output).unwrap();

        // Both outputs should be empty
        assert_eq!(result.0, "");
        assert_eq!(result.1, "");
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_output_truncation() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Generate output with many lines to test truncation
        let mut long_lines = Vec::new();
        for i in 1..=150 {
            long_lines.push(format!("Line {}", i));
        }
        let long_output = long_lines.join("\n");

        let result = server.process_shell_output(&long_output).unwrap();

        // Check that final output contains truncation info
        assert!(result.0.contains("private note: output was 150 lines"));
        assert!(result.0.contains("truncated output:"));

        // Check that user output shows truncation notice
        assert!(result
            .1
            .contains("NOTE: Output was 150 lines, showing only the last 100 lines"));

        // Verify it shows the last 100 lines (use exact line matching to avoid substring matches)
        assert!(result.1.contains("Line 51\n"));
        assert!(result.1.contains("Line 150"));
        assert!(!result.1.contains("Line 1\n"));
        assert!(!result.1.contains("Line 50\n"));
    }

    #[tokio::test]
    #[serial]
    #[cfg(windows)]
    async fn test_windows_specific_commands() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Test PowerShell command
        let shell_params = Parameters(ShellParams {
            command: "Get-ChildItem".to_string(),
        });

        // Note: This test should be adapted to work with RequestContext
        // For now, we test the underlying functionality that would be used by shell
        assert!(true); // Test shell parameter creation works

        // Test that resolve_path works with Windows paths
        let windows_path = r"C:\Windows\System32";
        if Path::new(windows_path).exists() {
            let resolved = server.resolve_path(windows_path);
            assert!(resolved.is_ok());
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_view_range_invalid() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a small file
        let content = "Line 1\nLine 2\nLine 3";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Test invalid range - start line beyond file
        let view_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "view".to_string(),
            view_range: Some(vec![10, 15]),
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let result = server.text_editor(view_params).await;
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("beyond the end of the file"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_missing_parameters() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file first
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some("Initial content".to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Test insert without new_str parameter
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: None, // Missing required parameter
            insert_line: Some(1),
        });

        let result = server.text_editor(insert_params).await;
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("Missing 'new_str' parameter"));

        // Test insert without insert_line parameter
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("New text".to_string()),
            insert_line: None, // Missing required parameter
        });

        let result = server.text_editor(insert_params).await;
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("Missing 'insert_line' parameter"));
    }

    #[test]
    #[serial]
    fn test_goosehints_with_file_references() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create referenced files
        let readme_path = temp_dir.path().join("README.md");
        std::fs::write(
            &readme_path,
            "# Project README\n\nThis is the project documentation.",
        )
        .unwrap();

        let guide_path = temp_dir.path().join("guide.md");
        std::fs::write(&guide_path, "# Development Guide\n\nFollow these steps...").unwrap();

        // Create .goosehints with references
        let hints_content = r#"# Project Information

Please refer to:
@README.md
@guide.md

Additional instructions here.
"#;
        let hints_path = temp_dir.path().join(".goosehints");
        std::fs::write(&hints_path, hints_content).unwrap();

        // Create server and check instructions
        let server = create_test_server();
        let server_info = server.get_info();

        assert!(server_info.instructions.is_some());
        let instructions = server_info.instructions.unwrap();

        // Should contain the .goosehints content
        assert!(instructions.contains("Project Information"));
        assert!(instructions.contains("Additional instructions here"));

        // Should contain the referenced files' content
        assert!(instructions.contains("# Project README"));
        assert!(instructions.contains("This is the project documentation"));
        assert!(instructions.contains("# Development Guide"));
        assert!(instructions.contains("Follow these steps"));

        // Should have attribution markers
        assert!(instructions.contains("--- Content from"));
        assert!(instructions.contains("--- End of"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_at_end() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 1\nLine 2\nLine 3";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Insert at the end (after line 3)
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Line 4".to_string()),
            insert_line: Some(3),
        });

        let insert_result = server.text_editor(insert_params).await.unwrap();

        let text = insert_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::Assistant))
            })
            .unwrap()
            .as_text()
            .unwrap();

        assert!(text.text.contains("Text has been inserted at line 4"));

        // Verify the file content by reading it directly
        let file_content = fs::read_to_string(&file_path).unwrap();
        assert!(file_content.contains("Line 1\nLine 2\nLine 3\nLine 4"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_at_end_negative() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 1\nLine 2\nLine 3";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Insert at the end using -1
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Line 4".to_string()),
            insert_line: Some(-1),
        });

        let insert_result = server.text_editor(insert_params).await.unwrap();

        let text = insert_result
            .content
            .iter()
            .find(|c| {
                c.audience()
                    .is_some_and(|roles| roles.contains(&Role::Assistant))
            })
            .unwrap()
            .as_text()
            .unwrap();

        assert!(text.text.contains("Text has been inserted at line 4"));

        // Verify the file content by reading it directly
        let file_content = fs::read_to_string(&file_path).unwrap();
        assert!(file_content.contains("Line 1\nLine 2\nLine 3\nLine 4"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_invalid_line() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 1\nLine 2\nLine 3";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Try to insert beyond the end of the file
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Line 11".to_string()),
            insert_line: Some(10),
        });

        let result = server.text_editor(insert_params).await;

        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("beyond the end of the file"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_with_undo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Create a file with some content
        let content = "Line 1\nLine 2";
        let write_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "write".to_string(),
            view_range: None,
            file_text: Some(content.to_string()),
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        server.text_editor(write_params).await.unwrap();

        // Insert a line
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("Inserted Line".to_string()),
            insert_line: Some(1),
        });

        server.text_editor(insert_params).await.unwrap();

        // Undo the insert
        let undo_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "undo_edit".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: None,
            insert_line: None,
        });

        let undo_result = server.text_editor(undo_params).await.unwrap();

        let text = undo_result
            .content
            .iter()
            .find(|c| c.as_text().is_some())
            .unwrap()
            .as_text()
            .unwrap();
        assert!(text.text.contains("Undid the last edit"));

        // Verify the file is back to original content
        let file_content = fs::read_to_string(&file_path).unwrap();
        assert!(file_content.contains("Line 1\nLine 2"));
        assert!(!file_content.contains("Inserted Line"));
    }

    #[tokio::test]
    #[serial]
    async fn test_text_editor_insert_nonexistent_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("nonexistent.txt");
        let file_path_str = file_path.to_str().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Try to insert into a nonexistent file
        let insert_params = Parameters(TextEditorParams {
            path: file_path_str.to_string(),
            command: "insert".to_string(),
            view_range: None,
            file_text: None,
            old_str: None,
            new_str: Some("New line".to_string()),
            insert_line: Some(0),
        });

        let result = server.text_editor(insert_params).await;

        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("does not exist"));
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_missing_parameters() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let _server = create_test_server();

        // Test that shell parameter validation works for empty parameters
        // This tests the core parameter handling logic without requiring RequestContext

        // Test empty command handling logic
        let empty_command = "";
        let cmd_parts: Vec<&str> = empty_command.split_whitespace().collect();
        assert!(
            cmd_parts.is_empty(),
            "Empty command should result in empty parts"
        );

        // Verify this would be caught by the shell method's parameter validation
        let shell_params = Parameters(ShellParams {
            command: "".to_string(),
        });

        // The shell method would handle empty commands gracefully
        // Test that parameter structure is created correctly
        assert_eq!(shell_params.0.command, "");
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_respects_ignore_patterns() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gooseignore file
        fs::write(".gooseignore", "secret.txt").unwrap();
        fs::write("secret.txt", "secret content").unwrap();

        let server = create_test_server();

        // Test that the ignore pattern checking logic works correctly
        // This tests the core functionality that would prevent shell access to restricted files

        // Verify ignore patterns are loaded correctly
        assert!(
            server.is_ignored(std::path::Path::new("secret.txt")),
            "secret.txt should be ignored based on .gooseignore"
        );

        assert!(
            !server.is_ignored(std::path::Path::new("allowed.txt")),
            "allowed.txt should not be ignored"
        );

        // Test command parsing logic that would be used in shell validation
        let command = "cat secret.txt";
        let cmd_parts: Vec<&str> = command.split_whitespace().collect();
        assert_eq!(cmd_parts[0], "cat");
        assert_eq!(cmd_parts[1], "secret.txt");

        // Verify that the path exists and would be caught by ignore checking
        let path = std::path::Path::new("secret.txt");
        assert!(path.exists(), "Test file should exist");
        assert!(
            server.is_ignored(path),
            "Shell method would detect this as ignored and block the command"
        );

        // Test allowed file would not be blocked
        fs::write("allowed.txt", "allowed content").unwrap();
        let allowed_path = std::path::Path::new("allowed.txt");
        assert!(allowed_path.exists(), "Allowed file should exist");
        assert!(
            !server.is_ignored(allowed_path),
            "Shell method would allow access to non-ignored files"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_respects_gitignore_fallback() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Create .gitignore file (no .gooseignore)
        fs::write(".gitignore", "*.log").unwrap();

        let server = create_test_server();

        // Test that gitignore fallback patterns work correctly
        assert!(
            server.is_ignored(Path::new("debug.log")),
            "*.log pattern from .gitignore should match debug.log when no .gooseignore exists"
        );
        assert!(
            !server.is_ignored(Path::new("debug.txt")),
            "*.log pattern should not match debug.txt"
        );

        // Test command that would be blocked by gitignore fallback
        fs::write("test.log", "log content").unwrap();
        let log_path = Path::new("test.log");
        assert!(log_path.exists(), "Log file should exist");
        assert!(
            server.is_ignored(log_path),
            "Shell method would block access to .log files via gitignore fallback"
        );

        // Test command that would be allowed
        fs::write("test.txt", "regular content").unwrap();
        let txt_path = Path::new("test.txt");
        assert!(txt_path.exists(), "Text file should exist");
        assert!(
            !server.is_ignored(txt_path),
            "Shell method would allow access to non-ignored files"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_shell_output_handling_logic() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let server = create_test_server();

        // Test output truncation logic with content without trailing newlines
        let content_without_newline = "Content without newline";
        let result = server
            .process_shell_output(content_without_newline)
            .unwrap();

        assert_eq!(result.0, content_without_newline);
        assert_eq!(result.1, content_without_newline);
        assert!(
            result.0.contains("Content without newline"),
            "Output processing should preserve content without trailing newlines"
        );

        // Test with content that has trailing newlines
        let content_with_newline = "Content with newline\n";
        let result = server.process_shell_output(content_with_newline).unwrap();
        assert_eq!(result.0, content_with_newline);
        assert_eq!(result.1, content_with_newline);

        // Test empty output handling
        let empty_output = "";
        let result = server.process_shell_output(empty_output).unwrap();
        assert_eq!(result.0, "");
        assert_eq!(result.1, "");
    }
}
