Class CustomerOne {
    ClassMethod callToSYS() {
        w "calling subclass one"
        d ##class(SubClassOne).newVarChange()
    }
}
