Class Demo.Person Extends %RegisteredObject
{
    Property Name As %String;

    Property Age As %Integer;

    ClassMethod %New() {

    }

    Method Introduce() As %String
    {
        Quit "Hi, my name is "_..Name_" and I am "_..Age_" years old."
    }

    ClassMethod Example()
    {
        Set person = ##class(Demo.Person).%New()

        Set person.Name = "Hannah"
        Set person.Age = 25

        Write person.Introduce(), !
    }
}
