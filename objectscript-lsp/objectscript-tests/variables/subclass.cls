Class SubClassOne Extends SuperClass {
    Method newVarChange() {
        set x = 7
        w x
        d ##class(SuperClass).multiplePubVarDefs()
    }
}
