Class SuperClass [Not ProcedureBlock, Language = objectscript]
{
    Method newVarChange() {
        set x = 32
        w x
        w y
        d ..multiplePubVarDefs()
    }

    ClassMethod multiplePubVarDefs()  {
        set y = 24
        w x
        w y
        write "Goodbye"
    }

    Method testingIndentation() {
        set y = 3
        set x = 12
        w y
    }
}
