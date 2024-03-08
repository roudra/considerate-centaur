package com.mdsol.ss.dt;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.apache.camel.builder.RouteBuilder;

import jakarta.enterprise.context.ApplicationScoped;

@ApplicationScoped
public class DTMysqlToMongoRoute extends RouteBuilder {

    private static final String KAFKA_SOURCE = "kafka:document-tracking?" +
            "brokers={{kafka.bootstrap.servers}}" +
            "&groupId=sim" +
            "&autoOffsetReset=earliest";
    private static final String MONGO_DESTINATION = "mongodb:dtMongoDB?" +
            "database=document-tracking" +
            "&collection=documents" +
            "&operation=save";
    private static final String ROUTE_ID = "document-tracking-kafka-mongo-route";

    @Override
    public void configure() {
        from(KAFKA_SOURCE).routeId(ROUTE_ID)
                .log("Received message header Kafka: ${headers}")
                .log("Received message from Kafka: ${body}")
                .process(exchange -> {
                    JsonNode jn = exchange.getMessage().getBody(JsonNode.class);
                    ((ObjectNode) jn).put("_id", jn.get("id"));
                    exchange.getMessage().setBody(jn.toString());
                })
                .log("Transformed Body ${body}")
                .to(MONGO_DESTINATION)
        ;
    }
}