Class hk {
    Relationship manyProp As User.hksubclass [ Cardinality = many, Inverse = oneProp ];
    ClassMethod print2() [Private] {
        w 2
    }
}
