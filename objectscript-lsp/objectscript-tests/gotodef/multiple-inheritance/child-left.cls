Class Demo.ChildDefault Extends (Demo.LeftParent, Demo.RightParent)
{
ClassMethod UseParent()
{
    do ##class(Demo.LeftParent).Shared()
     d ..Common()
}
}
