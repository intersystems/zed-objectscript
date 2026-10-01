Class SuperClass Extends ScopeResolution [Not ProcedureBlock]
{
    ClassMethod newVarChange() {
        set x = 32
        w x
        w y
        d ..multiplePubVarDefs()
    }

    ClassMethod multiplePubVarDefs()  {
        set y = 24
        zw
        w x
        w y
        write "Goodbye"
    }

    ClassMethod testingIndentation() {
        set y = 3
        set x = 12
        w y
    }
}
