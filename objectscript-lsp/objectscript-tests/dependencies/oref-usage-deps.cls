Class Demo.OrefUsageDependencies [ Not ProcedureBlock, Language = objectscript ]
{

Method BuildAndUsePublic() {
    Set obj = ##class(External.PublicOrefClass).%New()
    Do obj.Run()
}

Method BuildAndUsePrivate() [ ProcedureBlock = 1 ] {
    Set pObj = ##class(External.PrivateOrefClass).%New()
    Do pObj.Ping()
}

Method DefineButUnused() {
    Set neverUsed = ##class(External.UnusedOrefClass).%New()
    Quit
}
}
