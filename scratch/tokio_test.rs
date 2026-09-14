use tokio::sync::OnceCell;
fn main() {
    let _cell: OnceCell<()> = OnceCell::const_new();
}
