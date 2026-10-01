Class SubClassOne Extends SuperClass {
    ClassMethod newVarChange() [ProcedureBlock = 0] {
        set x = 7
        d ##class(SuperClass).multiplePubVarDefs()
        d ..m1()
    }
}
