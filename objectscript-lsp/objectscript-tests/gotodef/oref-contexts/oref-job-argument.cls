Class Demo.OrefJobArg
{
Method Launch() [ProcedureBlock = 0]
{
    set worker = ##class(Demo.Target).%New()
    job worker.Execute()
}
}
