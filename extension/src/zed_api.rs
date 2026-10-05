//! The Zed abstraction boundary.
//!
//! This is the ONLY module that knows about Zed-specific extension
//! limitations. Everything else in the extension speaks to the [`ZedHost`]
//! trait; if Zed later provides list pickers, native process APIs or a
//! remote-project API, only this implementation is replaced.

/// A remote project to open in Zed.
#[derive(Debug, Clone)]
pub struct RemoteTarget {
    /// Documented Zed CLI URL: `ssh://[user@]host[:port]/path`.
    pub ssh_url: String,
}

/// What the extension needs from its host environment.
pub trait ZedHost {
    /// Surface a message to the user.
    fn show_message(&self, message: &str);

    /// Ask the user to pick a value.
    ///
    /// LIMITATION (see ASSUMPTIONS.md): Zed's extension API has no picker or
    /// input primitive. Selections are modelled as slash-command argument
    /// completions, so this trait method is not implementable today; callers
    /// use the numbered-flow in `commands.rs` instead. Kept in the trait so
    /// the calling code does not change when Zed gains pickers.
    fn prompt(&self, prompt: &str) -> Result<String, String>;

    /// Ask Zed to open a remote project.
    ///
    /// LIMITATION (see ASSUMPTIONS.md): the extension API cannot open
    /// projects or windows. The WASM implementation delegates to the `zrd`
    /// native agent, which invokes the documented `zed ssh://...` CLI.
    fn open_remote_project(&self, target: RemoteTarget) -> Result<(), String>;
}

/// Messages in the WASM extension are delivered as slash-command output
/// (the only user-visible surface available to extensions).
pub struct WasmZedHost;

impl ZedHost for WasmZedHost {
    fn show_message(&self, _message: &str) {
        // No-op: messages are composed into SlashCommandOutput by callers.
    }

    fn prompt(&self, _prompt: &str) -> Result<String, String> {
        Err(
            "Zed extensions cannot show input prompts; use slash-command \
             argument completion instead."
                .into(),
        )
    }

    fn open_remote_project(&self, target: RemoteTarget) -> Result<(), String> {
        // The extension API cannot open remote projects itself, so the
        // hand-off goes through the native agent, which shells out to the
        // documented `zed ssh://...` CLI.
        crate::client::ZrdClient::default().launch_zed(&target.ssh_url)
    }
}
