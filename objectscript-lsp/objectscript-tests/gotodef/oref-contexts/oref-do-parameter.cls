Class Demo.OrefDoParam
{
Method Setup() [ProcedureBlock = 0]
{
    set obj = ##class(Demo.Target).%New()
    do obj.Run()
}
}
