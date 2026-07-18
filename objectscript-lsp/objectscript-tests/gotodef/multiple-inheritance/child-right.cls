Class Demo.ChildRight Extends (Demo.LeftParent, Demo.RightParent) [Inheritance = right]
{
Method UseParent()
{
    do ##class(Demo.RightParent).Shared()
}
}
