use crate::executor::Risk;

pub fn prompt() -> &'static str {
    "❯"
}
pub fn command_icon() -> &'static str {
    "⚙"
}
pub fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "✓ Safe",
        Risk::Caution => "⚠ Caution",
        Risk::Dangerous => "✗ Dangerous",
    }
}
pub async fn loader() {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut index = 0;
    loop {
        print!("\r{} thinking...", frames[index % frames.len()]);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        index += 1;
    }
}
