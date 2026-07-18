Class Demo.Target
{
ClassMethod %New() As Demo.Target
{
    Quit
}

Method Run() As %Status
{
    Write "running"
    Quit 1
}

Method Execute() As %Status
{
    Write "executing"
    Quit 1
}
}
