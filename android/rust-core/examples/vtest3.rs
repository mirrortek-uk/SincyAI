use sincy_adapter::schema::AdapterDefinition;
fn main() {
    let yaml = std::fs::read_to_string("adapters/x.reply_comment.yaml").unwrap();
    let c1 = serde_json::to_string(&AdapterDefinition::from_yaml(&yaml).unwrap()).unwrap();
    let data = std::fs::read_to_string("D:/tmp/reply_def.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&data).unwrap();
    let c2 =
        serde_json::to_string(&AdapterDefinition::from_yaml(&v["definition"].to_string()).unwrap())
            .unwrap();
    println!("equal: {}", c1 == c2);
    if c1 != c2 {
        println!("FILE: {}", &c1[..c1.len().min(400)]);
        println!("DB  : {}", &c2[..c2.len().min(400)]);
        for (a, b) in c1.bytes().zip(c2.bytes()) {
            if a != b {
                println!("first diff at byte");
                break;
            }
        }
        println!("len {} vs {}", c1.len(), c2.len());
    }
}
