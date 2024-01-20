# Telemetry Web UI

Steps to run (repeat after each code change)
1. cd to top of Pronghorn project
2. mvn clean install -DskipTests
3. mvn exec:java -Dexec.addResourcesToClasspath=true -Dexec.mainClass="com.javanut.pronghorn.TelemetryTestTool"
4. browse any of the three URLs output after "Telemetry Server is now ready on"

To test UI locally (for faster turnaround)
1. cd to src/main/resources/telemetry
2. npm install
3. npm run start
4. browse localhost:3000

