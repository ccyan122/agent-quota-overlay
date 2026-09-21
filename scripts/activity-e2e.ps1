param(
  [Parameter(Mandatory = $true)]
  [ValidateSet('active', 'idle')]
  [string]$Event,
  [ValidateSet('chatgpt_work', 'claude_code', 'antigravity')]
  [string]$Provider = 'claude_code',
  [string]$SessionId = 'manual_e2e'
)

# This invokes only the shipped bounded helper. Start the overlay first, then use
# a harmless synthetic lifecycle event to verify local named-pipe delivery.
$root = Split-Path -Parent $PSScriptRoot
$helper = Join-Path $root 'src-tauri\target\debug\llm-overlay-hook.exe'
if (-not (Test-Path -LiteralPath $helper)) {
  throw 'Build the helper first: cargo build --manifest-path src-tauri\Cargo.toml --bin llm-overlay-hook'
}
# The explicit bounded synthetic session skips stdin, so this test does not
# depend on the calling shell's pipeline behavior.
& $helper $Provider $Event $SessionId
if ($LASTEXITCODE -ne 0) { throw "Helper failed with exit code $LASTEXITCODE" }
Write-Output 'Helper exited successfully. Inspect the overlay indicator for the selected provider.'
