Class Demo.Caller
{
ClassMethod Main()
{
    do ##class(Demo.Utility).Helper()
    set result = ##class(Demo.Utility).Compute(1, 2)
    quit
}
}
