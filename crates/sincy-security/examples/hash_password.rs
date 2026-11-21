//! Ops tool: hash a password (Argon2id) for agency_members.password_hash.
//!   cargo run -p sincy-security --example hash_password
//! reads the password from stdin, prints the hash.
fn main() {
    use std::io::Read;
    let mut pw = String::new();
    std::io::stdin()
        .read_to_string(&mut pw)
        .expect("read stdin");
    let pw = pw.trim_end_matches(['\r', '\n']);
    let hash = sincy_security::password::hash_password(pw).expect("hash");
    println!("{hash}");
}
