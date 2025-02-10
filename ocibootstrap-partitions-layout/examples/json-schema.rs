use ocibootstrap_partitions_layout::PartitionTable;
use schemars::schema_for;
use serde as _;

fn main() {
    let schema = schema_for!(PartitionTable);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
