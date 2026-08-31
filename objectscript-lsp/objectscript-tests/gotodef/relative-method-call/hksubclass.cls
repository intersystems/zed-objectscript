Class hksubclass Extends (hk, %RegisteredObject) {
    Relationship oneProp As User.hk [ Cardinality = one, Inverse = manyProp, Required ];
    Property x As %Boolean [InitialExpression = 1];
    ClassMethod callInheritedMethod() {
        d ..print2()
        if 1 { d }
    }
}
