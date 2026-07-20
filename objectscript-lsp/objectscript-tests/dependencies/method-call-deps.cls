Class Demo.MethodCallDependencies Extends %RegisteredObject
{

Method CaptureCalls() As %Status
{
    Do ##class(External.ClassOne).Run(1)
    Set sc = $classmethod("External.ClassTwo","Ping",2)
    Set sc2 = $method("External.ClassThree","Pong",3)
    Do ..LocalSelf(4)
    Quit 1
}

Method LocalSelf(x As %Integer = 0) As %Status
{
    Quit 1
}

}
