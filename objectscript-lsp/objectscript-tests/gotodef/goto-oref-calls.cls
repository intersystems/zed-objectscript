Class Testing {
    Method CreateOref() {
        set x = ##class(Demo.Person).%New();
        d ..FindOref()
    }

    Method FindOref() [ProcedureBlock=0] {
        d x.Introduce()
    }
}
