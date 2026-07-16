Class Demo.QueryExamples Extends %RegisteredObject
{

/// 1) Basic class query (%SQLQuery)
Query EmployeesByCity(pCity As %String = "")
  As %SQLQuery(ROWSPEC="ID:%Integer,Name:%String,Dept:%String",CONTAINID=1)
  [ SqlProc, SqlName = Demo_EmployeesByCity ]
{
SELECT ID, Name, Department AS Dept
FROM Demo.Employee
WHERE (:pCity = '' OR City %STARTSWITH :pCity)
ORDER BY Name
}

/// 2) Custom class query (%Query + Execute/Fetch/Close)
Query ActivePatients(minAge As %Integer = 0)
  As %Query(ROWSPEC="ID:%Integer,Name:%String,Age:%Integer",CONTAINID=1)
{
}

ClassMethod ActivePatientsExecute(ByRef qHandle As %Binary, minAge As %Integer = 0) As %Status
{
    Set stmt=##class(%SQL.Statement).%New()
    Set sql="SELECT ID,Name,Age FROM Demo.Patient WHERE Age >= ? ORDER BY Name"
    Set sc=stmt.%Prepare(sql)
    If sc'=1 Quit sc
    Set qHandle=stmt.%Execute(minAge)
    Quit 1
}

ClassMethod ActivePatientsFetch(
    ByRef qHandle As %Binary,
    ByRef Row As %List,
    ByRef AtEnd As %Integer = 0
) As %Status [ PlaceAfter = ActivePatientsExecute ]
{
    Set rs=$Get(qHandle)
    If rs="" {
        Set Row="",AtEnd=1
        Quit 1
    }

    If rs.%Next() {
        Set Row=$ListBuild(rs.%Get("ID"),rs.%Get("Name"),rs.%Get("Age"))
        Set AtEnd=0
    } Else {
        Set Row="",AtEnd=1
    }

    Quit 1
}

ClassMethod ActivePatientsClose(ByRef qHandle As %Binary) As %Status [ PlaceAfter = ActivePatientsFetch ]
{
    Set qHandle=""
    Quit 1
}

/// 3) Embedded SQL example (&sql with cursor)
ClassMethod EmbeddedSqlCursor(state As %String = "MA") As %Status
{
    New SQLCODE,%ROWCOUNT,name,dob

    &sql(DECLARE C1 CURSOR FOR
         SELECT Name, DOB
         FROM Demo.Patient
         WHERE HomeState = :state
         ORDER BY Name)

    &sql(OPEN C1)

    For {
        &sql(FETCH C1 INTO :name,:dob)
        Quit:SQLCODE'=0
        Write !,name," | ",dob
    }

    &sql(CLOSE C1)
    Quit 1
}

/// 4) SQL Gateway style SQL via dynamic SQL text
ClassMethod SQLGatewayExample(city As %String = "Boston") As %Status
{
    Set ddlServer = "CREATE FOREIGN SERVER Demo.ExtDB FOREIGN DATA WRAPPER JDBC CONNECTION 'PostgresConnection'"
    Set ddlTable = "CREATE FOREIGN TABLE Demo.ExtPatients (PatientID BIGINT, Name VARCHAR(100), City VARCHAR(50)) SERVER Demo.ExtDB TABLE 'public.patients'"
    Set query = "SELECT PatientID,Name FROM Demo.ExtPatients WHERE City = ?"

    Set stmt = ##class(%SQL.Statement).%New()
    Set sc = stmt.%Prepare(query)
    If sc'=1 Quit sc
    Set rs = stmt.%Execute(city)
    Quit 1
}
}
