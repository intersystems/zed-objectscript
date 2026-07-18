Class Demo.ChildDefault Extends (Demo.LeftParent, Demo.RightParent)
{
Method UseParent()
{
    do ##class(Demo.LeftParent).Shared()
}
}
