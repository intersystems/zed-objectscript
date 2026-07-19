Class ScopeResolution [Not ProcedureBlock, Language = objectscript]
{

Method m1() {
    w x
}

Method m2() {
    set x = 200
    d ..m1()
}

Method m3() {
    set x = 300
    d ..m2()
}

Method conditionalScope() {
    set y = 1
    if (1 = 1) {
        set y = 2
    }
    set y = 3
    w y
}

Method sameScope() {
    set z = 10
    set z = 20
    w z
}

}
