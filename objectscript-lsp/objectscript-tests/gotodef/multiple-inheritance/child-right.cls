Class Demo.ChildRight Extends (Demo.LeftParent, Demo.RightParent) [Inheritance = right]
{
ClassMethod UseParent()
{
    do ##class(Demo.RightParent).Shared()
    d ..Common()
}
}
