use graphviz_rust::dot_generator::{graph, id};
use graphviz_rust::dot_structures::Graph;
use graphviz_rust::parse;
use structopt::StructOpt;


fn main() {



    println!("This is cargo-steady-init!");
    parse_test();
    // Your command logic here.

    //take the dot file and parse it
    //send to code generation for project construction
    //also generate svg?
    //can we validate this with LLM?

}


fn parse_test() {
    let g: Result<Graph, String> = parse(
        r#"
        strict digraph t {
            aa[color=green]
            subgraph v {
                aa[shape=square]
                subgraph vv{a2 -> b2}
                aaa[color=red]
                aaa -> bbb
            }
            aa -> be -> subgraph v { d -> aaa}
            aa -> aaa -> v
        }
        "#,
    );



/*
    assert_eq!(
        g,
        graph!(strict di id!("t");
          node!("aa";attr!("color","green")),
          subgraph!("v";
            node!("aa"; attr!("shape","square")),
            subgraph!("vv"; edge!(node_id!("a2") => node_id!("b2"))),
            node!("aaa";attr!("color","red")),
            edge!(node_id!("aaa") => node_id!("bbb"))
            ),
          edge!(node_id!("aa") => node_id!("be") => subgraph!("v"; edge!(node_id!("d") => node_id!("aaa")))),
          edge!(node_id!("aa") => node_id!("aaa") => node_id!("v"))
        )
    ) */
}